[Setup]
AppId=BetterExplorer
AppName=BetterExplorer
AppVersion={#AppVersion}
AppPublisher=Aziz Taban
AppPublisherURL=https://github.com/TabanAziz/BetterExplorer
DefaultDirName={localappdata}\Programs\BetterExplorer Setup
UsePreviousAppDir=no
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
MinVersion=10.0.22000
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
CloseApplications=no
SetupIconFile=..\assets\bex.ico
UninstallDisplayIcon={localappdata}\Programs\BetterExplorer\BetterExplorer.exe
UninstallDisplayName=BetterExplorer
OutputBaseFilename={#OutputBaseName}
WizardStyle=modern
Compression=lzma2
SolidCompression=yes

[Files]
Source: "{#PayloadDir}\install.ps1"; Flags: dontcopy
Source: "{#PayloadDir}\bundle-manifest.json"; Flags: dontcopy
Source: "{#PayloadDir}\README.md"; Flags: dontcopy
Source: "{#PayloadDir}\LICENSE"; Flags: dontcopy
Source: "{#PayloadDir}\THIRD-PARTY-LICENSES"; Flags: dontcopy
Source: "{#PayloadDir}\BetterExplorer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\install-common.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\uninstall.ps1"; DestDir: "{app}"; Flags: ignoreversion

[Run]
Filename: "{localappdata}\Programs\BetterExplorer\BetterExplorer.exe"; Description: "Open BetterExplorer"; Flags: nowait postinstall skipifsilent; Check: ManagedInstallSucceeded

[Code]
const
  WinEOption = 0;
  FolderManagerOption = 1;
  ShortcutOption = 2;

var
  Options: TInputOptionWizardPage;
  PreviousWinE: Boolean;
  PreviousFolderManager: Boolean;
  InstallError: String;

procedure RunScript(const Directory, Script, Options: String; var Output: TExecOutput);
var
  Params: String;
  ExitCode: Integer;
begin
  Params := '-NoProfile -NonInteractive -ExecutionPolicy RemoteSigned -File ' +
    AddQuotes(Directory + '\' + Script) + Options;
  if not ExecAndCaptureOutput(
    ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Params, Directory, SW_HIDE, ewWaitUntilTerminated, ExitCode, Output) then
    RaiseException('Could not start Windows PowerShell. ' + SysErrorMessage(ExitCode));

  if ExitCode <> 0 then
  begin
    if GetArrayLength(Output.StdErr) > 0 then
      RaiseException(Trim(StringJoin(#13#10, Output.StdErr)));
    RaiseException('BetterExplorer setup returned exit code ' + IntToStr(ExitCode) + '.');
  end;
end;

procedure InitializeWizard();
var
  Output: TExecOutput;
  Description: String;
begin
  ExtractTemporaryFile('install.ps1');
  ExtractTemporaryFile('install-common.ps1');
  ExtractTemporaryFile('uninstall.ps1');
  ExtractTemporaryFile('bundle-manifest.json');
  ExtractTemporaryFile('BetterExplorer.exe');
  ExtractTemporaryFile('README.md');
  ExtractTemporaryFile('LICENSE');
  ExtractTemporaryFile('THIRD-PARTY-LICENSES');

  if WizardSilent then
    Exit;

  RunScript(ExpandConstant('{tmp}'), 'install.ps1', ' -QueryOptions', Output);
  if Output.Error or (GetArrayLength(Output.StdOut) <> 3) then
    RaiseException('Could not read the current BetterExplorer setup options.');

  PreviousWinE := Output.StdOut[0] = 'enabled';
  PreviousFolderManager := Output.StdOut[1] = '0';
  Description := 'These options apply to your Windows account.';
  if Output.StdOut[0] = 'other' then
    Description := Description + #13#10 + 'Another copy of BetterExplorer manages Win+E.';
  if Output.StdOut[1] = '3' then
    Description := Description + #13#10 + 'Select the folder option to make BetterExplorer the default again.';
  if Output.StdOut[1] = '4' then
    Description := Description + #13#10 + 'Another registration manages the BetterExplorer folder command.';
  if Output.StdOut[2] = 'other' then
    Description := Description + #13#10 + 'A Start menu shortcut already uses the BetterExplorer name.';

  Options := CreateInputOptionPage(wpWelcome, 'BetterExplorer options',
    'Choose how BetterExplorer opens.', Description, False, False);
  Options.Add('Open BetterExplorer when I press Win+E');
  Options.Add('Use BetterExplorer as the default folder manager');
  Options.Add('Create a Start menu shortcut');
  Options.Values[WinEOption] := PreviousWinE;
  Options.Values[FolderManagerOption] := PreviousFolderManager;
  Options.Values[ShortcutOption] := Output.StdOut[2] <> 'other';
  Options.CheckListBox.ItemEnabled[WinEOption] := Output.StdOut[0] <> 'other';
  Options.CheckListBox.ItemEnabled[FolderManagerOption] := Output.StdOut[1] <> '4';
  Options.CheckListBox.ItemEnabled[ShortcutOption] := Output.StdOut[2] = 'available';
end;

function ManagedInstallSucceeded(): Boolean;
begin
  Result := InstallError = '';
end;

function GetInstallArguments(): String;
begin
  Result := ' -Quiet';
  if not WizardSilent then
  begin
    if Options.Values[WinEOption] <> PreviousWinE then
    begin
      if Options.Values[WinEOption] then
        Result := Result + ' -WinE BetterExplorer'
      else
        Result := Result + ' -WinE WindowsExplorer';
    end;
    if Options.Values[FolderManagerOption] <> PreviousFolderManager then
    begin
      if Options.Values[FolderManagerOption] then
        Result := Result + ' -FolderManager BetterExplorer'
      else
        Result := Result + ' -FolderManager WindowsExplorer';
    end;
    if Options.Values[ShortcutOption] and Options.CheckListBox.ItemEnabled[ShortcutOption] then
      Result := Result + ' -StartMenuShortcut';
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Output: TExecOutput;
begin
  Result := '';
  try
    RunScript(ExpandConstant('{tmp}'), 'install.ps1', GetInstallArguments() + ' -ValidateOnly', Output);
  except
    Result := GetExceptionMessage;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Output: TExecOutput;
begin
  { Deploy after Inno has installed the helpers and finalized the uninstaller. }
  if CurStep <> ssPostInstall then
    Exit;

  try
    RunScript(ExpandConstant('{tmp}'), 'install.ps1', GetInstallArguments(), Output);
  except
    InstallError := GetExceptionMessage;
    Log('BetterExplorer installation failed: ' + InstallError);
    SuppressibleMsgBox(InstallError, mbError, MB_OK, IDOK);
  end;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpFinished) and not ManagedInstallSucceeded() then
  begin
    WizardForm.FinishedHeadingLabel.Caption := 'BetterExplorer installation failed';
    WizardForm.FinishedLabel.Caption := 'Run Setup again to retry. To remove the setup files, use Installed apps in Windows Settings.';
    WizardForm.RunList.Visible := False;
  end;
end;

function GetCustomSetupExitCode(): Integer;
begin
  if ManagedInstallSucceeded() then
    Result := 0
  else
    Result := 9;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Output: TExecOutput;
begin
  if CurUninstallStep = usUninstall then
  begin
    try
      RunScript(ExpandConstant('{app}'), 'uninstall.ps1', '', Output);
    except
      SuppressibleMsgBox(GetExceptionMessage, mbError, MB_OK, IDOK);
      Abort;
    end;
  end;
end;
