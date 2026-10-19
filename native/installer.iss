[Setup]
AppId={{07B1698E-8815-43DA-B2DA-F90FE6310943}
AppName=NetTime Next Rust Preview
AppVersion=0.4.2
AppPublisher=NetTime Next Project
DefaultDirName={autopf}\NetTime Next Rust Preview
DefaultGroupName=NetTime Next Rust Preview
OutputDir=..\dist
OutputBaseFilename=NetTimeNext-Rust-0.4.2-Preview-Setup-x64
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
Name: "observer"; Description: "Run the native Windows service automatically (measurement mode)"

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
var ResultCode: Integer; JournalText: AnsiString;
begin
  Result := '';
  if FileExists(ExpandConstant('{commonappdata}\NetTimeNextNativePreview\clock-ownership.json')) then
    if (not LoadStringFromFile(ExpandConstant('{commonappdata}\NetTimeNextNativePreview\clock-ownership.json'), JournalText)) or (Pos('"phase": "rolled_back"', String(JournalText)) = 0) then begin
      Result := 'Restore previous clock services from Settings before upgrading.';
      exit;
    end;
  if FileExists(ExpandConstant('{app}\NetTimeNextNativeService.exe')) and RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\NetTimeNextNativePreview') then
    if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--stop-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
      Result := 'Restore previous clock services from Settings before upgrading. A service that cannot stop must be resolved before installation.';
end;
procedure CurStepChanged(CurStep: TSetupStep);
var ResultCode: Integer;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('observer') then begin
    if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--install-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
      RaiseException('Native service registration failed. The active clock service was not changed.');
    if (not Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--start-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
      RaiseException('Native service startup failed. See ProgramData\NetTimeNextNativePreview\data\service-error.log.');
  end;
end;
function InitializeUninstall(): Boolean;
var ResultCode: Integer;
begin
  if not RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\NetTimeNextNativePreview') then begin Result := True; exit; end;
  Result := Exec(ExpandConstant('{app}\NetTimeNextNativeService.exe'), '--remove-service', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
  if not Result then MsgBox('The service could not be safely removed. Restore previous clocks and retry.', mbError, MB_OK);
end;
