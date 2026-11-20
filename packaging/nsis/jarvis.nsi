; Jarvis for Windows installer. Built by packaging/windows/build.ps1.
; Per-user install under %LOCALAPPDATA%\Programs\Jarvis, no elevation required.

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!ifndef VERSION4
  !define VERSION4 "0.1.0.0"
!endif
!ifndef EXE
  !define EXE "..\..\target\release\jarvis.exe"
!endif
!ifndef OUT
  !define OUT "Jarvis-${VERSION}-x64-setup.exe"
!endif
!ifndef DML
  !define DML ""
!endif

!define APP_NAME "Jarvis"
!define APP_EXE "jarvis.exe"
!define APP_KEY "Software\Jarvis"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Jarvis"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define DATA_DIR "$LOCALAPPDATA\Jarvis"
!ifndef ICON
  !define ICON "${__FILEDIR__}/../windows/jarvis.ico"
!endif

Name "${APP_NAME}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Jarvis"
InstallDirRegKey HKCU "${APP_KEY}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Jarvis for Windows · local-first"

VIProductVersion "${VERSION4}"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "FileDescription" "Jarvis for Windows installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION4}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Apache-2.0"
VIAddVersionKey /LANG=1033 "OriginalFilename" "Jarvis-${VERSION}-x64-setup.exe"

!define MUI_ABORTWARNING
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_WELCOMEPAGE_TITLE "Jarvis for Windows"
!define MUI_WELCOMEPAGE_TEXT "Jarvis runs speech, intent and commands locally. Setup installs the app for this user; models download inside Jarvis after it starts."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Open Jarvis"
!define MUI_FINISHPAGE_LINK "Project documentation"
!define MUI_FINISHPAGE_LINK_LOCATION "https://github.com/CHITESHVARUN10/jarvuss"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Jarvis (required)" SEC_CORE
  SectionIn RO
  SetShellVarContext current

  ; Replace a running copy so the executable and DirectML runtime can update.
  nsExec::ExecToLog 'taskkill /IM ${APP_EXE} /F'
  Pop $0

  IfFileExists "$SYSDIR\vcruntime140.dll" vc_runtime_ok
  MessageBox MB_OK|MB_ICONINFORMATION "Jarvis uses the Microsoft Visual C++ runtime. If Jarvis does not start, install the Visual C++ 2015-2022 Redistributable (x64) from https://aka.ms/vs/17/release/vc_redist.x64.exe."
  vc_runtime_ok:

  SetOutPath "$INSTDIR"
  File "${EXE}"
  !if "${DML}" != ""
    File "${DML}"
  !endif
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Jarvis contributors"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  WriteRegStr HKCU "${APP_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${APP_KEY}" "Version" "${VERSION}"
SectionEnd

Section "Start Menu shortcut" SEC_STARTMENU
  SetShellVarContext current
  CreateDirectory "$SMPROGRAMS\Jarvis"
  CreateShortcut "$SMPROGRAMS\Jarvis\Jarvis.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  CreateShortcut "$SMPROGRAMS\Jarvis\Uninstall Jarvis.lnk" "$INSTDIR\uninstall.exe"
SectionEnd

Section "Desktop shortcut" SEC_DESKTOP
  SetShellVarContext current
  CreateShortcut "$DESKTOP\Jarvis.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
SectionEnd

Section "Uninstall"
  SetShellVarContext current

  nsExec::ExecToLog 'taskkill /IM ${APP_EXE} /F'
  Pop $0

  Delete "$SMPROGRAMS\Jarvis\Jarvis.lnk"
  Delete "$SMPROGRAMS\Jarvis\Uninstall Jarvis.lnk"
  RMDir "$SMPROGRAMS\Jarvis"
  Delete "$DESKTOP\Jarvis.lnk"

  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\DirectML.dll"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegValue HKCU "${RUN_KEY}" "Jarvis"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${APP_KEY}"

  MessageBox MB_YESNO|MB_DEFBUTTON2 "Remove Jarvis settings, routines, insights and downloaded models from this PC?" IDNO keep_data
  RMDir /r "${DATA_DIR}"
  keep_data:
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_CORE} "Jarvis application and local runtime."
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_STARTMENU} "Add Jarvis to the Start menu."
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_DESKTOP} "Add a Jarvis shortcut to the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END
