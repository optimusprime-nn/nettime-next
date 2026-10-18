#ifndef AppVersion
  #define AppVersion "0.3.3"
#endif

[Setup]
AppId={{346BB99F-C97D-4BC6-B66F-C4543C60B371}
AppName=NetTime Next
AppVersion={#AppVersion}
AppPublisher=NetTime Next Project
AppPublisherURL=https://github.com/Optimusprimeums/nettime-next
AppSupportURL=https://github.com/Optimusprimeums/nettime-next/issues
DefaultDirName={autopf}\NetTime Next
DefaultGroupName=NetTime Next
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=NetTimeNext-{#AppVersion}-Setup-x64
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
MinVersion=10.0
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\App\NetTimeNext.exe
CloseApplications=yes
RestartApplications=no
SetupLogging=yes
InfoBeforeFile=preview-notice.txt

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "..\dist\NetTimeNext\*"; DestDir: "{app}\App"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\dist\NetTimeNextService\*"; DestDir: "{app}\Service"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\*.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\THIRD-PARTY-LICENSES\*"; DestDir: "{app}\ThirdPartyLicenses"; Flags: ignoreversion

[Icons]
Name: "{group}\NetTime Next"; Filename: "{app}\App\NetTimeNext.exe"
Name: "{group}\Uninstall NetTime Next"; Filename: "{uninstallexe}"
Name: "{autodesktop}\NetTime Next"; Filename: "{app}\App\NetTimeNext.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\App\NetTimeNext.exe"; Description: "Open NetTime Next"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
function ServiceCommand(const Command: String): Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(ExpandConstant('{app}\Service\NetTimeNextService.exe'), Command,
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\Service\NetTimeNextService.exe')) then
    if not ServiceCommand('stop') then
      Result := 'The NetTime Next Observer service could not stop. Close the app and retry. The existing NetTime service is not changed.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    if not ServiceCommand('install') then
      RaiseException('Observer service registration failed. See the service log in ProgramData\NetTimeNext\data.');
    if not ServiceCommand('start') then
      RaiseException('Observer service startup failed. See the service log in ProgramData\NetTimeNext\data.');
  end;
end;

function InitializeUninstall(): Boolean;
begin
  Result := ServiceCommand('remove');
  if (not Result) and (not UninstallSilent) then
    MsgBox('The observer service could not be removed. Uninstall stopped to preserve its files.', mbError, MB_OK);
end;
