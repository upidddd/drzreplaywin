; Inno Setup script: per-user install (no admin rights) into %LOCALAPPDATA%\Programs\DrzReplayWIN
#define AppVersion GetEnv("APP_VERSION")

[Setup]
AppId={{B7C0F856-E567-4495-BC75-EAFB11562BFB}
AppName=DrzReplayWIN
AppVersion={#AppVersion}
AppVerName=DrzReplayWIN {#AppVersion}
AppPublisher=DRZ
AppPublisherURL=https://github.com/upidddd/DrzReplayWIN
AppSupportURL=https://github.com/upidddd/DrzReplayWIN/issues
DefaultDirName={localappdata}\Programs\DrzReplayWIN
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=DrzReplayWIN-Setup
SetupIconFile=..\src\drzreplaywin\assets\icons\drzreplaywin.ico
UninstallDisplayIcon={app}\DrzReplayWIN.exe
UninstallDisplayName=DrzReplayWIN
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=force
RestartApplications=no

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[InstallDelete]
; files of the previous version
Type: filesandordirs; Name: "{app}\_internal"

[Files]
Source: "..\dist\DrzReplayWIN\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\DrzReplayWIN"; Filename: "{app}\DrzReplayWIN.exe"; AppUserModelID: "DRZ.DrzReplayWIN"
Name: "{autodesktop}\DrzReplayWIN"; Filename: "{app}\DrzReplayWIN.exe"; AppUserModelID: "DRZ.DrzReplayWIN"; Tasks: desktopicon

[Registry]
; "Start with Windows" is written by the app; remove it on uninstall
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "DrzReplayWIN"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\DrzReplayWIN.exe"; Description: "Start DrzReplayWIN"; Flags: nowait postinstall; Check: not WizardSilent
Filename: "{app}\DrzReplayWIN.exe"; Parameters: "--updated"; Flags: nowait; Check: WizardSilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /t /im DrzReplayWIN.exe"; Flags: runhidden; RunOnceId: "StopApp"
