use better_explorer::model::from_wide;
use std::{
    ffi::c_void,
    process::Command,
    sync::{
        mpsc::{channel, Sender},
        Arc, Mutex,
    },
    time::Duration,
};
use windows::{
    core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, PCWSTR},
    Win32::{
        Foundation::{CLASS_E_NOAGGREGATION, CO_E_SERVER_STOPPING, E_FAIL, POINT},
        System::{
            Com::{
                CoAddRefServerProcess, CoInitializeEx, CoRegisterClassObject,
                CoReleaseServerProcess, CoResumeClassObjects, CoRevokeClassObject, CoTaskMemFree,
                CoUninitialize, IClassFactory, IClassFactory_Impl, IServiceProvider,
                CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, REGCLS_MULTIPLEUSE, REGCLS_SUSPENDED,
            },
            Ole::{IObjectWithSite, IObjectWithSite_Impl},
        },
        UI::Shell::{
            IExecuteCommand, IExecuteCommand_Impl, IObjectWithSelection, IObjectWithSelection_Impl,
            IShellBrowser, IShellItem, IShellItemArray, SHGetIDListFromObject, SID_SCommDlgBrowser,
            SID_SInPlaceBrowser, SBSP_ABSOLUTE, SBSP_DEFBROWSER, SBSP_NEWBROWSER,
            SIGDN_FILESYSPATH,
        },
    },
};

pub const CLSID: GUID = GUID::from_u128(0x801e1d22_334a_4b1f_9f3b_a99dc4a38721);
pub const SERVER_ARG: &str = "--folder-command-server";

pub fn run() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let result = serve();
    unsafe { CoUninitialize() };
    result
}

enum ServerState {
    Activated,
    Idle,
}

enum Startup {
    Waiting,
    Activated,
    Expired,
}

fn serve() -> Result<()> {
    let (lifetime, state) = channel();
    let startup = Arc::new(Mutex::new(Startup::Waiting));
    unsafe { CoAddRefServerProcess() };
    let startup_hold = ServerReference(lifetime.clone());
    let factory: IClassFactory = Factory {
        lifetime,
        startup: startup.clone(),
    }
    .into();
    let cookie = unsafe {
        CoRegisterClassObject(
            &CLSID,
            &factory,
            CLSCTX_LOCAL_SERVER,
            REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED,
        )?
    };
    let started = unsafe { CoResumeClassObjects() };
    if started.is_err() || state.recv_timeout(Duration::from_secs(30)).is_err() {
        let mut phase = startup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(*phase, Startup::Waiting) {
            *phase = Startup::Expired;
        }
    }
    drop(startup_hold);
    if started.is_ok() {
        while let Ok(ServerState::Activated) = state.recv() {}
    }
    let revoked = unsafe { CoRevokeClassObject(cookie) };
    started.and(revoked)
}

fn release_server(lifetime: &Sender<ServerState>) {
    // COM stops new activations when the last command and server lock are released.
    if unsafe { CoReleaseServerProcess() } == 0 {
        let _ = lifetime.send(ServerState::Idle);
    }
}

struct ServerReference(Sender<ServerState>);

impl Drop for ServerReference {
    fn drop(&mut self) {
        release_server(&self.0);
    }
}

#[implement(IClassFactory)]
struct Factory {
    lifetime: Sender<ServerState>,
    startup: Arc<Mutex<Startup>>,
}

impl Factory {
    fn retain_server(&self) -> Result<()> {
        let mut phase = self.startup.lock().map_err(|_| E_FAIL)?;
        if matches!(*phase, Startup::Expired) {
            return Err(CO_E_SERVER_STOPPING.into());
        }
        unsafe { CoAddRefServerProcess() };
        if matches!(*phase, Startup::Waiting) {
            *phase = Startup::Activated;
            let _ = self.lifetime.send(ServerState::Activated);
        }
        Ok(())
    }
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        self.retain_server()?;
        let command: IExecuteCommand = FolderCommand {
            site: Mutex::new(None),
            selection: Mutex::new(None),
            _server: ServerReference(self.lifetime.clone()),
        }
        .into();
        unsafe { command.query(iid, object).ok() }
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            self.retain_server()?;
        } else {
            release_server(&self.lifetime);
        }
        Ok(())
    }
}

#[implement(IExecuteCommand, IObjectWithSelection, IObjectWithSite)]
struct FolderCommand {
    site: Mutex<Option<IUnknown>>,
    selection: Mutex<Option<IShellItemArray>>,
    _server: ServerReference,
}

impl IObjectWithSite_Impl for FolderCommand_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> Result<()> {
        let site = site.as_ref().cloned();
        let previous = std::mem::replace(&mut *self.site.lock().map_err(|_| E_FAIL)?, site);
        drop(previous);
        Ok(())
    }

    fn GetSite(&self, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        let site = self
            .site
            .lock()
            .map_err(|_| E_FAIL)?
            .clone()
            .ok_or(E_FAIL)?;
        unsafe { site.query(iid, object).ok() }
    }
}

impl IObjectWithSelection_Impl for FolderCommand_Impl {
    fn SetSelection(&self, selection: Ref<IShellItemArray>) -> Result<()> {
        let selection = selection.as_ref().cloned();
        let previous =
            std::mem::replace(&mut *self.selection.lock().map_err(|_| E_FAIL)?, selection);
        drop(previous);
        Ok(())
    }

    fn GetSelection(&self, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        let selection = self
            .selection
            .lock()
            .map_err(|_| E_FAIL)?
            .clone()
            .ok_or(E_FAIL)?;
        unsafe { selection.query(iid, object).ok() }
    }
}

impl IExecuteCommand_Impl for FolderCommand_Impl {
    fn SetKeyState(&self, _: u32) -> Result<()> {
        Ok(())
    }

    fn SetParameters(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn SetPosition(&self, _: &POINT) -> Result<()> {
        Ok(())
    }

    fn SetShowWindow(&self, _: i32) -> Result<()> {
        Ok(())
    }

    fn SetNoShowUI(&self, _: BOOL) -> Result<()> {
        Ok(())
    }

    fn SetDirectory(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn Execute(&self) -> Result<()> {
        let site = self.site.lock().map_err(|_| E_FAIL)?.clone();
        let selection = self
            .selection
            .lock()
            .map_err(|_| E_FAIL)?
            .clone()
            .ok_or(E_FAIL)?;
        let count = unsafe { selection.GetCount()? };
        if let Some(browser) = site.as_ref().and_then(explorer_browser) {
            let mode = if count == 1 {
                SBSP_DEFBROWSER
            } else {
                SBSP_NEWBROWSER
            };
            for index in 0..count {
                let item = unsafe { selection.GetItemAt(index)? };
                browse(&browser, &item, mode)?;
            }
            return Ok(());
        }

        let exe = std::env::current_exe()?;
        for index in 0..count {
            let item = unsafe { selection.GetItemAt(index)? };
            let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
            let path = from_wide(unsafe { name.as_wide() });
            unsafe { CoTaskMemFree(Some(name.as_ptr().cast())) };
            Command::new(&exe).arg("--open-folder").arg(path).spawn()?;
        }
        Ok(())
    }
}

fn explorer_browser(site: &IUnknown) -> Option<IShellBrowser> {
    let services = site.cast::<IServiceProvider>().ok()?;
    let browser = unsafe { services.QueryService(&SID_SInPlaceBrowser).ok()? };
    // Query the invocation site directly. Browser window lookups can fail during an
    // input-synchronous call from File Explorer.
    if unsafe { services.QueryService::<IShellBrowser>(&SID_SCommDlgBrowser) }.is_ok() {
        return None;
    }
    Some(browser)
}

fn browse(browser: &IShellBrowser, item: &IShellItem, mode: u32) -> Result<()> {
    let pidl = unsafe { SHGetIDListFromObject(item)? };
    let result = unsafe { browser.BrowseObject(pidl, SBSP_ABSOLUTE | mode) };
    unsafe { CoTaskMemFree(Some(pidl.cast())) };
    result
}
