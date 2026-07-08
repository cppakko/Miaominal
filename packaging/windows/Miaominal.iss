#define AppName "Miaominal"
#define AppPublisher "cppakko"
#define AppDescription "Desktop terminal for SSH, SFTP, and secure configuration management."
#define AppUrl "https://github.com/cppakko/miaominal"
#define AppUpdateUrl "https://github.com/cppakko/miaominal/releases"

[Setup]
AppId={{2657DCFA-4ECC-41B2-9AC7-26E91BB5C137}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
AppUpdatesURL={#AppUpdateUrl}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
LicenseFile={#LicenseFile}
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile={#ProductIcon}
UninstallDisplayIcon={app}\miaominal.exe
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimp"; MessagesFile: "languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#BinarySource}"; DestDir: "{app}"; DestName: "miaominal.exe"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#AppName}\{#AppName}"; Filename: "{app}\miaominal.exe"; WorkingDir: "{app}"; Comment: "{#AppDescription}"
Name: "{userprograms}\{#AppName}\Uninstall {#AppName}"; Filename: "{uninstallexe}"; WorkingDir: "{app}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\miaominal.exe"; WorkingDir: "{app}"; Comment: "{#AppDescription}"; Tasks: desktopicon

[Run]
Filename: "{app}\miaominal.exe"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Registry]
Root: HKCU; Subkey: "Software\cppakko\Miaominal"; ValueType: dword; ValueName: "Installed"; ValueData: 1; Flags: uninsdeletekeyifempty
