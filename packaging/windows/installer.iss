; Inno Setup Script for OxiFind
; Generates a signed, modern Windows Installer with Context Menu & PATH integration

#define MyAppName "OxiFind"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "OxiFind Project"
#define MyAppURL "https://github.com/oxifind/oxifind"
#define MyAppExeName "oxifind.exe"

[Setup]
AppId={{D8E53A8B-276C-4B9F-8417-6B4B1FA93D2E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputDir=..\..\target\release\installer
OutputBaseFilename=OxiFind-{#MyAppVersion}-Setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add OxiFind to PATH environment variable"; GroupDescription: "System integration:"
Name: "contextmenu"; Description: "Add 'Search with OxiFind' to folder context menu"; GroupDescription: "Explorer integration:"

[Files]
Source: "..\..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Explorer context menu for folders
Root: HKCU; Subkey: "Software\Classes\Directory\shell\OxiFind"; ValueType: string; ValueName: ""; ValueData: "Search with OxiFind"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\OxiFind"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"""; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\OxiFind\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

; Explorer context menu for drive roots
Root: HKCU; Subkey: "Software\Classes\Drive\shell\OxiFind"; ValueType: string; ValueName: ""; ValueData: "Search with OxiFind"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\OxiFind"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"""; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\OxiFind\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

; PATH environment variable update (User level when non-elevated)
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}')); Tasks: addtopath

[Run]
Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Filename: "{app}\{#MyAppExeName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Clean up local application data and snapshots on uninstallation if opted
Type: files; Name: "{localappdata}\OxiFind\index.snapshot"
Type: files; Name: "{localappdata}\OxiFind\settings.json"
Type: dirifempty; Name: "{localappdata}\OxiFind"

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath)
  then begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + UpperCase(Param) + ';', ';' + UpperCase(OrigPath) + ';') = 0;
  if Result then
    Result := Pos(';' + UpperCase(Param), ';' + UpperCase(OrigPath)) = 0;
end;
