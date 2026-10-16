; Tauri NSIS hooks (PRD §12.2.3): the installer is per-user and never ships VB-CABLE.

!macro NSIS_HOOK_PREINSTALL
  ; An upgrade replaces the binaries: stop the running daemon first; it restores on its way out.
  IfFileExists "$INSTDIR\macafon.exe" 0 +2
    ExecWait '"$INSTDIR\macafon.exe" ctl quit'
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
