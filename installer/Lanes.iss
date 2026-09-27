; Lanes - installer script for Inno Setup 6.
;
; Build it with:
;     powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
;
; which runs tools\build-release.ps1 first, so the installer always packages a
; freshly built dist\ rather than whatever happened to be lying there.
;
; ---------------------------------------------------------------------------
; WHY THIS INSTALLS PER USER AND NEVER ASKS FOR ADMINISTRATOR
; ---------------------------------------------------------------------------
;
; Lanes only ever writes to HKCU and to %LOCALAPPDATA%, and anything that
; needed elevation would mean the approach had gone wrong somewhere. An
; installer that asked for admin would be the app quietly acquiring a
; privilege it has no use for.
;
; PrivilegesRequired=lowest puts it in %LOCALAPPDATA%\Programs\Lanes, which the
; user owns. No UAC prompt, no service, no driver, no COM registration, nothing
; machine-wide. Uninstalling is a matter of removing one folder and one
; registry value.
;
; ---------------------------------------------------------------------------
; WHAT THIS DOES NOT DO
; ---------------------------------------------------------------------------
;
; It does not install a runtime, because there is nothing to install: the core
; links the C runtime statically and the window carries .NET inside its own
; executable. It does not write the Run key either - "start with Windows" is a
; setting inside the app, which owns that value so it can correct the path if
; the program is ever moved. An installer writing it too would mean two owners
; of one registry value and no way to tell which was right.
;
; If you fork Lanes, change AppId, AppUserId, the names and the URL below, or
; your installer will upgrade - and replace - an installed Lanes.

#define AppName        "Lanes"
#define AppExe         "Lanes.exe"
#define WindowExe      "Lanes.Window.exe"
; The identity Windows groups and pins this application under. It must match
; the string the mixer window sets on itself - see ui-wpf/Api/CoreProcess.cs -
; or pinning the running window pins Lanes.Window.exe, which starts a mixer
; with no core behind it. Never change it once shipped: an existing pin records
; it, and a different one silently stops matching.
#define AppUserId      "MAYBE-33.Lanes"

#define AppVersion     "1.0.0"
#define AppPublisher   "MAYBE-33"
#define AppUrl         "https://github.com/MAYBE-33/lanes"

[Setup]
; What Windows uses to decide whether this is an upgrade or a second product.
; Never change it: a new id would install beside an existing Lanes instead of
; upgrading it.
AppId={{8E2B7A14-3C5D-4E9F-9B21-6D4A0F7C2E88}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppCopyright=Copyright (C) 2026 {#AppPublisher}. Licensed under the GPL, version 3 or later.
VersionInfoVersion={#AppVersion}

; Per user. See the note at the top of this file.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=no

; x64 only: the interop Lanes depends on is built for it, and offering a
; 32-bit install that cannot work would be worse than refusing.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763

OutputDir=..\dist\installer
OutputBaseFilename=Lanes-{#AppVersion}-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}

Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: "startup"; Description: "Start {#AppName} when I sign in"; \
    GroupDescription: "After installing:"

[Files]
Source: "..\dist\{#AppExe}";    DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\{#WindowExe}"; DestDir: "{app}"; Flags: ignoreversion
; The GPL asks that anyone receiving the program also receives the licence.
Source: "..\LICENSE";                  DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.md";   DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\troubleshooting.md";  DestDir: "{app}"; DestName: "Troubleshooting.md"; Flags: ignoreversion

; Every shortcut that starts the app points at the CORE, never at the window,
; and carries the AppUserModelID the window sets on itself. That pairing is
; what makes Windows pin this shortcut - and so start the whole application -
; when somebody pins the mixer window they are looking at.
[Icons]
Name: "{group}\{#AppName}";   Filename: "{app}\{#AppExe}"; AppUserModelID: "{#AppUserId}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; AppUserModelID: "{#AppUserId}"; Tasks: desktopicon

[Run]
; --minimised goes to the tray without opening the window, which is what
; someone who ticked "start when I sign in" is asking for the rest of the time.
Filename: "{app}\{#AppExe}"; Parameters: "--minimised"; Tasks: startup; \
    Description: "Start {#AppName} now"; Flags: nowait postinstall skipifsilent
Filename: "{app}\{#AppExe}"; Tasks: not startup; \
    Description: "Start {#AppName} now"; Flags: nowait postinstall skipifsilent

[Code]
{ ------------------------------------------------------------------------
  Ask a running copy to quit rather than failing halfway through.

  Windows will not replace a file that is open, so an upgrade with the tray
  still running would leave one new executable and one old one - which is
  exactly the state that makes a bug report impossible to read.
  ------------------------------------------------------------------------ }
function InitializeSetup(): Boolean;
var
  ResultCode: Integer;
  Exe: String;
begin
  Result := True;
  Exe := ExpandConstant('{localappdata}\Programs\{#AppName}\{#AppExe}');

  { Harmless when none is running: the command finds no instance and returns. }
  if FileExists(Exe) then
  begin
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    Sleep(600);
  end;
end;

{ ------------------------------------------------------------------------
  Stop the app before anything is deleted.

  It is tray-resident, so an uninstall with it still running cannot remove its
  own executable: Windows will not delete a file that is open.

  Not an [UninstallRun] entry, which looks right and is not: that would run
  "Lanes.exe --quit" from the very file being deleted, and even with
  waituntilterminated the image can still be mapped when the deletion is
  attempted - an uninstall that reports success and leaves Lanes.exe behind.

  Doing it here, in InitializeUninstall, puts the whole thing before Inno's
  file-removal step. The extra second is because a process exiting and its
  image being released are not the same instant.

  --quit is a clean shutdown rather than a kill: Lanes holds COM interfaces
  for its entire lifetime and must release them in order, and it owns
  config.json, whose write is atomic but not instantaneous.
  ------------------------------------------------------------------------ }
function InitializeUninstall(): Boolean;
var
  ResultCode: Integer;
  Exe: String;
begin
  Result := True;
  Exe := ExpandConstant('{app}\{#AppExe}');

  if FileExists(Exe) then
  begin
    { Harmless when nothing is running: --quit finds no instance, says so, and
      returns zero. }
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    Sleep(1500);
  end;
end;

{ ------------------------------------------------------------------------
  Uninstalling leaves the user's settings alone unless they say otherwise.

  %LOCALAPPDATA%\Lanes holds the channel layout, the rules, the audit log and
  the snapshots - including the record of Windows' default device and volumes
  from before Lanes changed them, which Restore uses. Deleting that by default
  would throw away the only record of how to put the machine back.
  ------------------------------------------------------------------------ }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  StateDir: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  StateDir := ExpandConstant('{localappdata}\Lanes');
  if not DirExists(StateDir) then
    Exit;

  { A silent uninstall never deletes anything it was not explicitly told to.

    A plain MsgBox under /SUPPRESSMSGBOXES returns the DEFAULT button - which
    for MB_YESNO is Yes - so an unattended uninstall would silently destroy the
    user's channels, rules and snapshots. Two rules follow, and both are
    applied below:
      - a destructive prompt's suppressed default must be the SAFE answer, not
        the first button;
      - a fully silent run must not take a destructive branch at all, because
        nobody is there to have answered. }
  if UninstallSilent() then
    Exit;

  { SuppressibleMsgBox, not MsgBox: the last argument is what it returns when
    prompts are suppressed, and here that is No. }
  if SuppressibleMsgBox('Remove your channels, rules and settings as well?' + #13#10#13#10 +
                        'This also deletes the record of your original Windows audio ' +
                        'settings. If you have not used "Restore Windows audio settings" ' +
                        'yet, choose No, reinstall, and restore first.',
                        mbConfirmation, MB_YESNO, IDNO) = IDYES then
    DelTree(StateDir, True, True, True);
end;
