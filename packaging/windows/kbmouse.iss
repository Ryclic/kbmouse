; Per-user installation keeps automatic updates writable without UAC elevation.
#ifndef AppVersion
  #error AppVersion must be supplied by scripts/package.py
#endif
#ifndef BinaryPath
  #error BinaryPath must be supplied by scripts/package.py
#endif
[Setup]
AppId={{6E2C44C9-309C-44C1-A68A-A96A4B6C3319}
AppName=kbmouse
AppVersion={#AppVersion}
AppPublisher=Ryclic
AppPublisherURL=https://github.com/Ryclic/kbmouse
DefaultDirName={localappdata}\Programs\kbmouse
DefaultGroupName=kbmouse
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
AppMutex=Local\kbmouse-single-instance
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\kbmouse.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; Flags: ignoreversion
[Icons]
Name: "{group}\kbmouse"; Filename: "{app}\kbmouse.exe"
Name: "{autodesktop}\kbmouse"; Filename: "{app}\kbmouse.exe"; Tasks: desktopicon
[Run]
Filename: "{app}\kbmouse.exe"; Description: "Launch kbmouse"; Flags: nowait postinstall skipifsilent
