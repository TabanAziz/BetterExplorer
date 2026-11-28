use std::{
    io,
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt,
        io::{FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows::{
    core::{Error, Interface, BSTR, PCWSTR},
    Win32::{
        Foundation::{RPC_E_CHANGED_MODE, VARIANT_FALSE, VARIANT_TRUE},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            TaskScheduler::{
                IActionCollection, IExecAction, ILogonTrigger, IRegisteredTask, ITaskDefinition,
                ITaskFolder, ITaskService, TaskScheduler, TASK_ACTION_EXEC, TASK_ACTION_TYPE,
                TASK_CREATE, TASK_INSTANCES_IGNORE_NEW, TASK_LOGON_INTERACTIVE_TOKEN,
                TASK_LOGON_TYPE, TASK_RUNLEVEL_LUA, TASK_RUNLEVEL_TYPE, TASK_TRIGGER_LOGON,
                TASK_TRIGGER_TYPE2, TASK_UPDATE,
            },
            Variant::VARIANT,
        },
    },
};
use windows_sys::Win32::{
    Foundation::{LocalFree, ERROR_INSUFFICIENT_BUFFER},
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, LookupAccountNameW, TokenUser,
        PSID, SECURITY_MAX_SID_SIZE, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const SOURCE: &str = "BetterExplorer.WinE.Logon.v1";
const LISTENER_ARG: &str = "--win-e-listener";
const DEFAULT_TASK_PRIORITY: i32 = 7;
const INTERACTIVE_TASK_PRIORITY: i32 = 5;

#[cfg(test)]
#[test]
fn only_the_scheduler_default_priority_is_migrated() {
    assert!(should_migrate_priority(true, DEFAULT_TASK_PRIORITY));
    assert!(!should_migrate_priority(false, DEFAULT_TASK_PRIORITY));
    assert!(!should_migrate_priority(true, 6));
    assert!(!should_migrate_priority(true, INTERACTIVE_TASK_PRIORITY));
}

fn should_migrate_priority(enabled: bool, priority: i32) -> bool {
    enabled && priority == DEFAULT_TASK_PRIORITY
}

pub fn ensure(exe: &Path) -> io::Result<()> {
    Scheduler::connect()?.ensure(exe, true)
}

pub fn ensure_for_update(exe: &Path) -> io::Result<()> {
    Scheduler::connect()?.ensure(exe, false)
}

pub fn remove(exe: &Path) -> io::Result<()> {
    Scheduler::connect()?.remove(exe)
}

pub fn scheduled(exe: &Path) -> io::Result<bool> {
    Scheduler::connect()?.scheduled(exe)
}

struct Scheduler {
    service: ITaskService,
    folder: ITaskFolder,
    sid: BSTR,
    name: BSTR,
    _com: ComApartment,
}

impl Scheduler {
    fn connect() -> io::Result<Self> {
        let sid = current_user_sid()?;
        let name = BSTR::from(format!("BetterExplorer Win+E Logon {sid}"));
        let com = ComApartment::initialize().map_err(io_error)?;
        let empty = VARIANT::default();
        unsafe {
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(io_error)?;
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(io_error)?;
            let folder = service.GetFolder(&BSTR::from("\\")).map_err(io_error)?;
            Ok(Self {
                service,
                folder,
                sid,
                name,
                _com: com,
            })
        }
    }

    fn lookup(&self) -> io::Result<Option<IRegisteredTask>> {
        match unsafe { self.folder.GetTask(&self.name) }.map_err(io_error) {
            Ok(task) => Ok(Some(task)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn ensure(&self, exe: &Path, enable_existing: bool) -> io::Result<()> {
        if let Some(task) = self.lookup()? {
            let definition = unsafe { task.Definition() }.map_err(io_error)?;
            if !self.owns(&definition, exe).map_err(io_error)? {
                return Err(name_in_use());
            }
            if !self.starts_at_logon(&definition).map_err(io_error)? {
                if enable_existing {
                    return Err(io::Error::other(
                        "The BetterExplorer logon trigger has been changed.",
                    ));
                }
                return Ok(());
            }
            let enabled = unsafe { task.Enabled() }.map_err(io_error)? != VARIANT_FALSE;
            let settings = unsafe { definition.Settings() }.map_err(io_error)?;
            let mut priority = 0;
            unsafe { settings.Priority(&mut priority) }.map_err(io_error)?;
            if should_migrate_priority(enabled || enable_existing, priority) {
                unsafe { settings.SetPriority(INTERACTIVE_TASK_PRIORITY) }.map_err(io_error)?;
                if !enabled {
                    unsafe { settings.SetEnabled(VARIANT_TRUE) }.map_err(io_error)?;
                }
                self.register(&definition, TASK_UPDATE.0)?;
            } else if enable_existing && !enabled {
                unsafe { task.SetEnabled(VARIANT_TRUE) }.map_err(io_error)?;
            }
            return Ok(());
        }

        let definition = self.definition(exe).map_err(io_error)?;
        // Create-only registration also protects a task added after the lookup.
        self.register(&definition, TASK_CREATE.0)
    }

    fn register(&self, definition: &ITaskDefinition, flags: i32) -> io::Result<()> {
        let empty = VARIANT::default();
        unsafe {
            self.folder.RegisterTaskDefinition(
                &self.name,
                definition,
                flags,
                &empty,
                &empty,
                TASK_LOGON_INTERACTIVE_TOKEN,
                &empty,
            )
        }
        .map_err(io_error)?;
        Ok(())
    }

    fn remove(&self, exe: &Path) -> io::Result<()> {
        let Some(task) = self.lookup()? else {
            return Ok(());
        };
        let definition = unsafe { task.Definition() }.map_err(io_error)?;
        if !self.owns(&definition, exe).map_err(io_error)? {
            if references_exe(&definition, exe).map_err(io_error)? {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "A modified logon task still references this BetterExplorer executable.",
                ));
            }
            return Ok(());
        }
        match unsafe { self.folder.DeleteTask(&self.name, 0) }.map_err(io_error) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    fn scheduled(&self, exe: &Path) -> io::Result<bool> {
        let Some(task) = self.lookup()? else {
            return Ok(false);
        };
        let definition = unsafe { task.Definition() }.map_err(io_error)?;
        Ok(self.owns(&definition, exe).map_err(io_error)?
            && self.starts_at_logon(&definition).map_err(io_error)?
            && unsafe { task.Enabled() }.map_err(io_error)? != VARIANT_FALSE)
    }

    fn definition(&self, exe: &Path) -> windows::core::Result<ITaskDefinition> {
        unsafe {
            let definition = self.service.NewTask(0)?;
            let info = definition.RegistrationInfo()?;
            info.SetSource(&BSTR::from(SOURCE))?;
            info.SetDescription(&BSTR::from(
                "Starts BetterExplorer's Win+E listener when you sign in.",
            ))?;

            let principal = definition.Principal()?;
            principal.SetUserId(&self.sid)?;
            principal.SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN)?;
            principal.SetRunLevel(TASK_RUNLEVEL_LUA)?;

            let settings = definition.Settings()?;
            settings.SetEnabled(VARIANT_TRUE)?;
            settings.SetPriority(INTERACTIVE_TASK_PRIORITY)?;
            settings.SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW)?;
            settings.SetDisallowStartIfOnBatteries(VARIANT_FALSE)?;
            settings.SetStopIfGoingOnBatteries(VARIANT_FALSE)?;
            settings.SetRunOnlyIfIdle(VARIANT_FALSE)?;
            settings.IdleSettings()?.SetStopOnIdleEnd(VARIANT_FALSE)?;
            settings.SetExecutionTimeLimit(&BSTR::from("PT0S"))?;

            let trigger: ILogonTrigger =
                definition.Triggers()?.Create(TASK_TRIGGER_LOGON)?.cast()?;
            trigger.SetUserId(&self.sid)?;
            trigger.SetEnabled(VARIANT_TRUE)?;

            let action: IExecAction = definition.Actions()?.Create(TASK_ACTION_EXEC)?.cast()?;
            action.SetPath(&exe_path(exe))?;
            action.SetArguments(&BSTR::from(LISTENER_ARG))?;
            Ok(definition)
        }
    }

    fn owns(&self, definition: &ITaskDefinition, exe: &Path) -> windows::core::Result<bool> {
        unsafe {
            let mut source = BSTR::new();
            definition.RegistrationInfo()?.Source(&mut source)?;
            if source != SOURCE {
                return Ok(false);
            }

            let principal = definition.Principal()?;
            let mut user = BSTR::new();
            let mut logon_type = TASK_LOGON_TYPE::default();
            let mut run_level = TASK_RUNLEVEL_TYPE::default();
            principal.UserId(&mut user)?;
            principal.LogonType(&mut logon_type)?;
            principal.RunLevel(&mut run_level)?;
            if logon_type != TASK_LOGON_INTERACTIVE_TOKEN
                || run_level != TASK_RUNLEVEL_LUA
                || !self.is_current_user(&user)
            {
                return Ok(false);
            }

            let actions = definition.Actions()?;
            let mut count = 0;
            actions.Count(&mut count)?;
            if count != 1 {
                return Ok(false);
            }
            let Some(action) = exec_action_at(&actions, 1)? else {
                return Ok(false);
            };
            let mut path = BSTR::new();
            let mut arguments = BSTR::new();
            let mut working_dir = BSTR::new();
            action.Path(&mut path)?;
            action.Arguments(&mut arguments)?;
            action.WorkingDirectory(&mut working_dir)?;
            Ok(path == exe_path(exe) && arguments == LISTENER_ARG && working_dir.is_empty())
        }
    }

    fn starts_at_logon(&self, definition: &ITaskDefinition) -> windows::core::Result<bool> {
        unsafe {
            let triggers = definition.Triggers()?;
            let mut count = 0;
            triggers.Count(&mut count)?;
            if count != 1 {
                return Ok(false);
            }
            let trigger = triggers.get_Item(1)?;
            let mut trigger_type = TASK_TRIGGER_TYPE2::default();
            trigger.Type(&mut trigger_type)?;
            if trigger_type != TASK_TRIGGER_LOGON {
                return Ok(false);
            }
            let trigger: ILogonTrigger = trigger.cast()?;
            let mut user = BSTR::new();
            let mut delay = BSTR::new();
            let mut enabled = VARIANT_FALSE;
            trigger.UserId(&mut user)?;
            trigger.Delay(&mut delay)?;
            trigger.Enabled(&mut enabled)?;
            Ok(enabled != VARIANT_FALSE
                && (delay.is_empty() || delay == "PT0S")
                && self.is_current_user(&user))
        }
    }

    fn is_current_user(&self, user: &BSTR) -> bool {
        // Task Scheduler can return the account name even when the SID was supplied.
        user == &self.sid || account_sid(user).is_ok_and(|sid| sid == self.sid)
    }
}

fn references_exe(definition: &ITaskDefinition, exe: &Path) -> windows::core::Result<bool> {
    let expected = exe_path(exe);
    unsafe {
        let actions = definition.Actions()?;
        let mut count = 0;
        actions.Count(&mut count)?;
        for index in 1..=count {
            if let Some(action) = exec_action_at(&actions, index)? {
                let mut path = BSTR::new();
                action.Path(&mut path)?;
                let path = path
                    .strip_prefix(&[b'"' as u16])
                    .and_then(|path| path.strip_suffix(&[b'"' as u16]))
                    .unwrap_or(&path);
                if CompareStringOrdinal(
                    path.as_ptr(),
                    path.len() as i32,
                    expected.as_ptr(),
                    expected.len() as i32,
                    1,
                ) == CSTR_EQUAL
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

fn exec_action_at(
    actions: &IActionCollection,
    index: i32,
) -> windows::core::Result<Option<IExecAction>> {
    unsafe {
        let action = actions.get_Item(index)?;
        let mut action_type = TASK_ACTION_TYPE::default();
        action.Type(&mut action_type)?;
        if action_type == TASK_ACTION_EXEC {
            Ok(Some(action.cast()?))
        } else {
            Ok(None)
        }
    }
}

fn current_user_sid() -> io::Result<BSTR> {
    const BUFFER_SIZE: usize = size_of::<TOKEN_USER>() + SECURITY_MAX_SID_SIZE as usize;
    let mut token = null_mut();
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _token = OwnedHandle::from_raw_handle(token);
        // The token buffer must align with TOKEN_USER and hold the SID it points to.
        let mut info = [0usize; BUFFER_SIZE.div_ceil(size_of::<usize>())];
        let mut length = 0;
        if GetTokenInformation(
            token,
            TokenUser,
            info.as_mut_ptr().cast(),
            size_of_val(&info) as u32,
            &mut length,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        sid_string((*info.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}

fn account_sid(account: &BSTR) -> io::Result<BSTR> {
    let mut sid_size = 0;
    let mut domain_size = 0;
    let mut name_use = 0;
    unsafe {
        LookupAccountNameW(
            null(),
            account.as_ptr(),
            null_mut(),
            &mut sid_size,
            null_mut(),
            &mut domain_size,
            &mut name_use,
        );
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }

        let mut sid = vec![0usize; (sid_size as usize).div_ceil(size_of::<usize>())];
        let mut domain = vec![0u16; domain_size as usize];
        if LookupAccountNameW(
            null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut name_use,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        sid_string(sid.as_mut_ptr().cast())
    }
}

unsafe fn sid_string(sid: PSID) -> io::Result<BSTR> {
    let mut string = null_mut();
    if ConvertSidToStringSidW(sid, &mut string) == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = BSTR::from_wide(PCWSTR(string).as_wide());
    LocalFree(string.cast());
    Ok(result)
}

fn exe_path(exe: &Path) -> BSTR {
    BSTR::from_wide(&exe.as_os_str().encode_wide().collect::<Vec<_>>())
}

fn name_in_use() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "The BetterExplorer logon task name is already in use.",
    )
}

fn io_error(error: Error) -> io::Error {
    let code = error.code().0 as u32;
    if code >> 16 == 0x8007 {
        io::Error::from_raw_os_error((code & 0xffff) as i32)
    } else {
        io::Error::other(error)
    }
}

struct ComApartment(bool);

impl ComApartment {
    fn initialize() -> windows::core::Result<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result == RPC_E_CHANGED_MODE {
            // COM already belongs to this thread in another apartment model.
            return Ok(Self(false));
        }
        result.ok()?;
        Ok(Self(true))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires access to the local Windows Task Scheduler"]
    fn registers_current_user_task_and_preserves_a_different_executable() -> io::Result<()> {
        let mut scheduler = Scheduler::connect()?;
        let fixture = tempfile::tempdir()?;
        scheduler.name = BSTR::from(format!(
            "BetterExplorer Win+E Fixture {}",
            fixture.path().file_name().unwrap().to_string_lossy()
        ));
        // The fixture can be registered without launching an executable.
        let exe = fixture.path().join("Test & Explorer.exe");
        let other_exe = fixture.path().join("Other.exe");
        assert!(!scheduler.scheduled(&exe)?);
        scheduler.ensure(&exe, false)?;
        struct Cleanup<'a>(&'a Scheduler);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                unsafe {
                    let _ = self.0.folder.DeleteTask(&self.0.name, 0);
                }
            }
        }
        let _cleanup = Cleanup(&scheduler);

        assert!(scheduler.scheduled(&exe)?);
        assert!(!scheduler.scheduled(&other_exe)?);
        assert_eq!(
            scheduler.ensure(&other_exe, true).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        scheduler.remove(&other_exe)?;
        assert!(scheduler.scheduled(&exe)?);
        unsafe { scheduler.lookup()?.unwrap().SetEnabled(VARIANT_FALSE) }.map_err(io_error)?;
        assert!(!scheduler.scheduled(&exe)?);
        scheduler.ensure(&exe, false)?;
        assert!(!scheduler.scheduled(&exe)?);
        scheduler.ensure(&exe, true)?;
        assert!(scheduler.scheduled(&exe)?);
        scheduler.remove(&exe)?;
        assert!(!scheduler.scheduled(&exe)?);
        scheduler.remove(&exe)?;

        // The second action runs the same Windows path, with different casing and arguments.
        (|| unsafe {
            let definition = scheduler.definition(&other_exe)?;
            let action: IExecAction = definition.Actions()?.Create(TASK_ACTION_EXEC)?.cast()?;
            action.SetPath(&exe_path(&fixture.path().join("test & explorer.EXE")))?;
            action.SetArguments(&BSTR::from("--different"))?;
            let empty = VARIANT::default();
            scheduler.folder.RegisterTaskDefinition(
                &scheduler.name,
                &definition,
                TASK_CREATE.0,
                &empty,
                &empty,
                TASK_LOGON_INTERACTIVE_TOKEN,
                &empty,
            )?;
            Ok::<_, Error>(())
        })()
        .map_err(io_error)?;
        assert_eq!(
            scheduler.remove(&exe).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(scheduler.lookup()?.is_some());
        Ok(())
    }
}
