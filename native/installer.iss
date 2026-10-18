[Setup]
AppId={{07B1698E-8815-43DA-B2DA-F90FE6310943}
AppName=NetTime Next Rust Preview
AppVersion=0.4.0
AppPublisher=NetTime Next Project
DefaultDirName={autopf}\NetTime Next Rust Preview
DefaultGroupName=NetTime Next Rust Preview
OutputDir=..\dist
OutputBaseFilename=NetTimeNext-Rust-0.4.0-Preview-Setup-x64
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
Name: "observer"; Description: "Register the separate Rust measurement service (manual start)"; Flags: unchecked

[Dirs]
Name: "{commonappdata}\NetTimeNextNativePreview"; Permissions: users-readexec
Name: "{commonappdata}\NetTimeNextNativePreview\data"; Permissions: users-readexec service-modify

[Files]
Source: "target\release\NetTimeNextNative.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\NetTimeNextNativeService.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\settings.example.json"; DestDir: "{commonappdata}\NetTimeNextNativePreview"; DestName: "settings.json"; Flags: onlyifdoesntexist uninsneveruninstall
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\third-party-licenses\*"; DestDir: "{app}\ThirdPartyLicenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\NetTime Next Rust Preview"; Filename: "{app}\NetTimeNextNative.exe"
Name: "{group}\Uninstall Rust Preview"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\NetTimeNextNative.exe"; Description: "Open the Rust desktop preview"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var ResultCode: Integer;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\NetTimeNextNativeService.exe')) then
    Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--stop-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;
procedure CurStepChanged(CurStep: TSetupStep);
var ResultCode: Integer;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('observer') then
    if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--install-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
      RaiseException('Native service registration failed. The active clock service was not changed.');
end;
function InitializeUninstall(): Boolean;
var ResultCode: Integer;
begin
  Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--stop-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--remove-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := True;
end;
