; Tauri NSIS hooks (PRD §12.2.3): the installer is per-user and never ships VB-CABLE.

; Set when an upgrade stopped a running daemon, so it comes back with the new binaries.
Var MacafonDaemonWasRunning

!include "nsDialogs.nsh"
!include "LogicLib.nsh"
Var MacafonKeepModels
Var MacafonKeepModelsCheckbox

; Registered before Tauri's uninstall pages. Silent uninstall defaults to deletion.
UninstPage custom un.MacafonModelsPage un.MacafonModelsPageLeave

Function un.MacafonModelsPage
  IfSilent macafon_models_page_done
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 32u "Downloaded dictation models will be deleted when Macafon is uninstalled."
  Pop $0
  ${NSD_CreateCheckbox} 0 40u 100% 16u "Keep downloaded dictation models"
  Pop $MacafonKeepModelsCheckbox
  ${If} $MacafonKeepModels == 1
    ${NSD_Check} $MacafonKeepModelsCheckbox
  ${EndIf}
  nsDialogs::Show
  macafon_models_page_done:
FunctionEnd

Function un.MacafonModelsPageLeave
  ${NSD_GetState} $MacafonKeepModelsCheckbox $MacafonKeepModels
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  ; An upgrade replaces the binaries: stop the running daemon first; it restores on its way
  ; out. `ctl quit` only asks, so wait until the running image no longer locks macafond.exe.
  StrCpy $MacafonDaemonWasRunning 0
  IfFileExists "$INSTDIR\macafond.exe" 0 macafon_stopped
    ClearErrors
    FileOpen $0 "$INSTDIR\macafond.exe" a
    IfErrors 0 macafon_unlocked
    StrCpy $MacafonDaemonWasRunning 1
    ExecWait '"$INSTDIR\macafon.exe" ctl quit'
    StrCpy $1 0
  macafon_wait:
    Sleep 250
    ClearErrors
    FileOpen $0 "$INSTDIR\macafond.exe" a
    IfErrors 0 macafon_unlocked
    IntOp $1 $1 + 1
    IntCmp $1 60 0 macafon_wait 0
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Macafon is still running. Quit it from the tray icon and click Retry." /SD IDCANCEL IDRETRY macafon_wait
    SetErrorLevel 1
    Abort
  macafon_unlocked:
    FileClose $0
  macafon_stopped:
  ; The UI stays warm in the background and leaves when the daemon does; give it a moment
  ; so its binary can be replaced (Tauri's own running-app check follows).
  StrCpy $1 0
  macafon_ui_wait:
    IfFileExists "$INSTDIR\macafon-ui.exe" 0 macafon_ui_stopped
    ClearErrors
    FileOpen $0 "$INSTDIR\macafon-ui.exe" a
    IfErrors 0 macafon_ui_unlocked
    Sleep 250
    IntOp $1 $1 + 1
    IntCmp $1 20 macafon_ui_stopped macafon_ui_wait macafon_ui_stopped
  macafon_ui_unlocked:
    FileClose $0
  macafon_ui_stopped:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; An upgrade keeps the shortcuts, so Explorer would keep showing the old icon from its
  ; cache: SHCNE_ASSOCCHANGED makes it reload them.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Bring back a daemon the upgrade stopped: the virtual microphone must not stay silent.
  ; Started through Explorer like Tauri's /R, so it does not belong to the installer's process tree.
  StrCmp $MacafonDaemonWasRunning 1 0 +2
    nsis_tauri_utils::RunAsUser "$INSTDIR\macafond.exe" "run"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Give back the default microphone and output and the app volumes, remove autostart
  ; (WIN-MIC-005, WIN-CH-008). VB-CABLE stays: remove it from its own setup if unwanted.
  IfFileExists "$INSTDIR\macafond.exe" 0 macafon_cleanup_done
    ClearErrors
    ExecWait '"$INSTDIR\macafond.exe" cleanup' $0
    IfErrors macafon_cleanup_failed
    IntCmp $0 0 macafon_cleanup_done macafon_cleanup_failed macafon_cleanup_failed
  macafon_cleanup_failed:
    ; A device that is gone for good must not block removal: let the user decide. Silent
    ; uninstalls stop, so nothing is left changed without anyone seeing it.
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "Macafon could not restore your audio settings (default microphone, output or app volumes). Reconnect your audio devices and try again, or check them in Windows Sound settings afterwards. Uninstall anyway?" /SD IDNO IDYES macafon_cleanup_done
    SetErrorLevel 1
    Abort
  macafon_cleanup_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $MacafonKeepModels != 1
    ; Custom storage folders are deliberately left alone: interpreting arbitrary
    ; TOML paths in NSIS would risk deleting files outside Macafon's model tree.
    RMDir /r "$LOCALAPPDATA\Macafon\models\stt"
  ${EndIf}
!macroend
