; Windows installer for superfastnotepad (NSIS 3).
;
; Installs for the current user (no admin prompt) into
; %LOCALAPPDATA%\Programs\superfastnotepad: the app and the console program
; superfastnotepad-cli.exe, a Start menu shortcut, an optional desktop
; shortcut, an "Open with" entry, App Paths entries (so `start` and the Run
; box find both by name), the install folder on the user's PATH (optional),
; and an uninstaller listed in Settings > Apps.
;
; Build (works on Windows, macOS or Linux with `makensis`):
;   makensis -DVERSION=0.3.5 -DEXE=<absolute path to superfastnotepad.exe> \
;            -DCLIEXE=<absolute path to superfastnotepad-cli.exe> \
;            -DOUTFILE=superfastnotepad-v0.3.5-windows-x64-setup.exe \
;            installer/superfastnotepad.nsi

!ifndef VERSION
  !error "pass -DVERSION=x.y.z"
!endif
!ifndef EXE
  !error "pass -DEXE=path to superfastnotepad.exe"
!endif
!ifndef CLIEXE
  !error "pass -DCLIEXE=path to superfastnotepad-cli.exe"
!endif
!ifndef OUTFILE
  !define OUTFILE "superfastnotepad-v${VERSION}-windows-x64-setup.exe"
!endif

!define APP "superfastnotepad"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define APP_KEY "Software\Classes\Applications\${APP}.exe"
!define APP_PATHS "Software\Microsoft\Windows\CurrentVersion\App Paths"

; PowerShell that adds or removes $INSTDIR in the user's PATH. It edits the
; registry value directly so entries like %USERPROFILE%\bin stay unexpanded
; and a long PATH isn't cut short.
!define PS 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command'
!define PS_ENV "$$k=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment');$$p=[string]$$k.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames);$$d='$INSTDIR';$$x=[Microsoft.Win32.RegistryValueKind]::ExpandString;"
!define PS_ADD "if(($$p -split ';') -notcontains $$d){$$k.SetValue('Path',($$p.TrimEnd(';')+';'+$$d).TrimStart(';'),$$x)}"
!define PS_REMOVE "if(($$p -split ';') -contains $$d){$$k.SetValue('Path',(($$p -split ';')|?{$$_ -and $$_ -ne $$d}) -join ';',$$x)}"

Unicode true
SetCompressor /SOLID lzma
ManifestDPIAware true

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel user

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License"

!include "MUI2.nsh"
!include "WinMessages.nsh"

!if /FileExists "../assets/windows/app.ico"
  !define MUI_ICON "../assets/windows/app.ico"
  !define MUI_UNICON "../assets/windows/app.ico"
!endif

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP}.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP}"

!insertmacro MUI_PAGE_LICENSE "../LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "${APP}" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  ; Close a running copy first so the .exe can be replaced on upgrade.
  nsExec::Exec 'taskkill /IM "${APP}.exe" /F'
  File "/oname=${APP}.exe" "${EXE}"
  File "/oname=${APP}-cli.exe" "${CLIEXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${APP}.exe"

  ; "Open with" for any file.
  WriteRegStr HKCU "${APP_KEY}" "FriendlyAppName" "${APP}"
  WriteRegStr HKCU "${APP_KEY}\DefaultIcon" "" '"$INSTDIR\${APP}.exe",0'
  WriteRegStr HKCU "${APP_KEY}\shell\open\command" "" '"$INSTDIR\${APP}.exe" "%1"'

  ; `start superfastnotepad`, the Run box and PowerShell's Start-Process find
  ; both programs by name.
  WriteRegStr HKCU "${APP_PATHS}\${APP}.exe" "" "$INSTDIR\${APP}.exe"
  WriteRegStr HKCU "${APP_PATHS}\${APP}.exe" "Path" "$INSTDIR"
  WriteRegStr HKCU "${APP_PATHS}\${APP}-cli.exe" "" "$INSTDIR\${APP}-cli.exe"
  WriteRegStr HKCU "${APP_PATHS}\${APP}-cli.exe" "Path" "$INSTDIR"

  ; Settings > Apps.
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" '"$INSTDIR\${APP}.exe",0'
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/darshan5/superfastnotepad"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Add to PATH" SecPath
  nsExec::Exec `${PS} "${PS_ENV}${PS_ADD}"`
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${APP}.exe"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The app, superfastnotepad-cli.exe (command line tools for scripts and AI agents), a Start menu shortcut and an $\"Open with$\" entry."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecPath} "Lets cmd and PowerShell run superfastnotepad and superfastnotepad-cli by name (in windows opened after installing)."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "A shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  nsExec::Exec 'taskkill /IM "${APP}.exe" /F'
  Delete "$INSTDIR\${APP}.exe"
  Delete "$INSTDIR\${APP}-cli.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegKey HKCU "${APP_KEY}"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${APP_PATHS}\${APP}.exe"
  DeleteRegKey HKCU "${APP_PATHS}\${APP}-cli.exe"
  nsExec::Exec `${PS} "${PS_ENV}${PS_REMOVE}"`
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  ; Settings and sessions live in the user's AppData and are kept.
SectionEnd
