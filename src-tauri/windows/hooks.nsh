; Extra steps for the Windows installer (see bundle > windows > nsis > installerHooks).
;
; Each idled game runs in its own steam-helper.exe, which keeps that file and
; steam_api64.dll open, and the installer cannot replace a file that is in use.
; The helpers exit on their own once IdleKit closes, but that can lag a moment
; behind the installer, so they are stopped here before any file is copied.

!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM steam-helper.exe'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM steam-helper.exe'
  Pop $0
!macroend
