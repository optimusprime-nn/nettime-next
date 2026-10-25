[Setup]
AppId={{07B1698E-8815-43DA-B2DA-F90FE6310943}
AppName=NetTime Next
AppVersion=1.0.1
AppPublisher=NetTime Next Project
DefaultDirName={autopf}\NetTime Next
DefaultGroupName=NetTime Next
OutputDir=..\dist
OutputBaseFilename=NetTimeNext-1.0.1-Setup-x64
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
MinVersion=10.0
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\NetTimeNextNative.exe
SetupLogging=yes

[Tasks]
Name: "observer"; Description: "Start the Windows service automatically (measurement mode)"

Name: "tray"; Description: "Start in the system tray when signing in"

[Dirs]
Name: "{commonappdata}\NetTimeNextNativePreview"; Permissions: users-readexec
Name: "{commonappdata}\NetTimeNextNativePreview\data"; Permissions: users-readexec service-modify

[Files]
Source: "target\release\NetTimeNextNative.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\NetTimeNextNativeService.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "settings.default.json"; DestDir: "{commonappdata}\NetTimeNextNativePreview"; DestName: "settings.json"; Flags: onlyifdoesntexist uninsneveruninstall
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\third-party-licenses\*"; DestDir: "{app}\ThirdPartyLicenses"; Flags: ignoreversion recursesubdirs createallsubdirs


[InstallDelete]
Type: files; Name: "{userstartup}\NetTime Next Rust.lnk"; Tasks: tray

[Icons]
Name: "{group}\NetTime Next"; Filename: "{app}\NetTimeNextNative.exe"
Name: "{group}\Uninstall NetTime Next"; Filename: "{uninstallexe}"

Name: "{commonstartup}\NetTime Next"; Filename: "{app}\NetTimeNextNative.exe"; Parameters: "--tray"; WorkingDir: "{app}"; IconFilename: "{app}\NetTimeNextNative.exe"; Tasks: tray

[Run]
Filename: "{app}\NetTimeNextNative.exe"; Description: "Open NetTime Next"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
var UpgradingExisting: Boolean;
function PrepareToInstall(var NeedsRestart: Boolean): String;
var ResultCode: Integer;
begin
  Result := '';
  UpgradingExisting := RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\NetTimeNextNativePreview');
  if UpgradingExisting then begin
    ExtractTemporaryFile('NetTimeNextNativeService.exe');
    if (not Exec(ExpandConstant('{tmp}\NetTimeNextNativeService.exe'), '--prepare-upgrade', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
      Result := 'The native collector could not safely pause for upgrade. Existing clock ownership was not transferred. See ProgramData\NetTimeNextNativePreview\admin-result.json.';
  end;
end;
procedure CurStepChanged(CurStep: TSetupStep);
var ResultCode: Integer;
begin
  if CurStep = ssPostInstall then begin
    if UpgradingExisting then begin
      if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--finish-upgrade', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
        RaiseException('The upgraded collector could not resume. Settings and clock ownership remain saved; review admin-result.json.');
    end else if WizardIsTaskSelected('observer') then begin
      if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--install-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
        RaiseException('Native service registration failed.');
      if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--start-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
        RaiseException('Native service startup failed. See ProgramData\NetTimeNextNativePreview\data\service-error.log.');
    end;
  end;
end;
function InitializeUninstall(): Boolean;
var ResultCode: Integer;
begin
  if not RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\NetTimeNextNativePreview') then begin Result := True; exit; end;
  Result := Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--remove-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
  if not Result then MsgBox('The service could not be safely removed. Restore previous clocks and retry.', mbError, MB_OK);
end;
