; Windows installer (Inno Setup 6), built by the release workflow:
;   ISCC /DAppVersion=<version> /DSourceDir=<unpacked package> /DOutputDir=<dir> /DOutputBase=<name>
; Installs for the current user only (no administrator rights), in %LOCALAPPDATA%\Programs.
; Settings and scores live in %APPDATA%\crack-attack and are kept on uninstall.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Never change the AppId: Windows uses it to recognise upgrades and the uninstaller.
AppId={{8F3C2A41-6B7D-4E2F-9A1C-5D0E7B3F4A21}
AppName=Crack Attack!
AppVersion={#AppVersion}
AppVerName=Crack Attack! {#AppVersion}
AppPublisher=Thiosaur
AppPublisherURL=https://github.com/thiosaur/crackattack
AppSupportURL=https://github.com/thiosaur/crackattack/issues
DefaultDirName={autopf}\Crack Attack
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBase}
SetupIconFile={#SourceDir}\AppIcon.ico
UninstallDisplayIcon={app}\AppIcon.ico
UninstallDisplayName=Crack Attack!
LicenseFile={#SourceDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; An upgrade replaces the game data entirely: a file removed from a newer version must not linger.
Type: filesandordirs; Name: "{app}\assets"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\Crack Attack!"; Filename: "{app}\crackattack.exe"; WorkingDir: "{app}"; IconFilename: "{app}\AppIcon.ico"
Name: "{autodesktop}\Crack Attack!"; Filename: "{app}\crackattack.exe"; WorkingDir: "{app}"; IconFilename: "{app}\AppIcon.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\crackattack.exe"; Description: "{cm:LaunchProgram,Crack Attack!}"; Flags: nowait postinstall skipifsilent
