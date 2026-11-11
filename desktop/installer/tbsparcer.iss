; The native client's installer. Built by tools/build-installer.ps1.
;
; Per user, so an update started from inside the program needs no elevation.
; It installs beside the Electron client and shares nothing with it.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z
#endif
#ifndef BuildDir
  #error Pass /DBuildDir=<cargo release folder>
#endif
#ifndef OutputDirectory
  #error Pass /DOutputDirectory=<folder>
#endif

[Setup]
AppId={{6F0E3B52-8C41-4B7E-9A55-2D3C7A1E4F10}
AppName=TBS inform CRM
AppVersion={#AppVersion}
AppPublisher=TBS inform
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\TBS inform CRM
DefaultGroupName=TBS inform CRM
; An update keeps its folder, but takes the new name in the Start menu.
UsePreviousGroup=no
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDirectory}
; Installed clients look for their update under this name, so it stays.
OutputBaseFilename=TBSparcer-Native-Setup-{#AppVersion}
SetupIconFile=..\crates\tbs-app\assets\app.ico
UninstallDisplayIcon={app}\tbsparcer.exe
UninstallDisplayName=TBS inform CRM
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#BuildDir}\tbsparcer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildDir}\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion
; The same file the Electron build ships. It holds the system access token, so
; it stays out of git, and the setup built from it must be treated the same way.
#ifexist "..\..\frontend\client-access.json"
Source: "..\..\frontend\client-access.json"; DestDir: "{app}\security"; Flags: ignoreversion
#endif

[InstallDelete]
; The shortcuts of the versions that were called TBSparcer.
Type: filesandordirs; Name: "{autoprograms}\TBSparcer"
Type: files; Name: "{autodesktop}\TBSparcer.lnk"

[Icons]
Name: "{group}\TBS inform CRM"; Filename: "{app}\tbsparcer.exe"
Name: "{autodesktop}\TBS inform CRM"; Filename: "{app}\tbsparcer.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tbsparcer.exe"; Description: "{cm:LaunchProgram,TBS inform CRM}"; Flags: nowait postinstall skipifsilent
