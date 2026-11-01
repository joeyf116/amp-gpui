; Builds the Windows installer. CI passes the version and output folder:
;   iscc /DAppVersion=0.2.0 /O..\dist packaging\amp-gpui.iss
; Paths are relative to this file. The release binary and license file have to
; be built first, as .github/workflows/release.yml does.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Never change AppId: it's how a new installer finds and upgrades an old install.
AppId={{5B7C2E1A-8D34-4F6E-9A12-6E0F3C7D4B21}
AppName=Amp GPUI
AppVersion={#AppVersion}
AppPublisher=joeyf116
AppPublisherURL=https://joeyf116.github.io/amp-gpui/
AppSupportURL=https://github.com/joeyf116/amp-gpui/issues
AppUpdatesURL=https://joeyf116.github.io/amp-gpui/download/
DefaultDirName={autopf}\Amp GPUI
DefaultGroupName=Amp GPUI
DisableProgramGroupPage=yes
; Per-user by default, so installing doesn't need an admin prompt.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=amp-gpui-setup
LicenseFile=..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\amp-gpui.exe
UninstallDisplayName=Amp GPUI
CloseApplications=yes

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "..\target\release\amp-gpui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\THIRD-PARTY-LICENSES.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\Amp GPUI"; Filename: "{app}\amp-gpui.exe"
Name: "{autodesktop}\Amp GPUI"; Filename: "{app}\amp-gpui.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\amp-gpui.exe"; Description: "Start Amp GPUI"; Flags: nowait postinstall skipifsilent

; Settings and tones live in %APPDATA%, and uninstalling leaves them there.
