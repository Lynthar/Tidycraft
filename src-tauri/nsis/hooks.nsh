; Puts the install directory on PATH so the bundled `tidycraft` CLI can be run
; from a terminal, a CI step or a local agent without opening the desktop app.
; The MSI does the same through Windows Installer's Environment table.
;
; The edit goes through PowerShell's registry API rather than NSIS's own
; ReadRegStr / WriteRegStr: NSIS strings stop at 1024 characters in the stock
; build, so reading a longer PATH and writing it back silently truncates it —
; which is how installers destroy a machine's PATH. PowerShell has no such cap.
;
; The script it writes is idempotent in both directions: it drops any existing
; entry for this directory first, then re-adds it unless -Remove was passed. So
; a repair or an upgrade cannot leave two copies behind, and an uninstall
; leaves the rest of PATH exactly as it was.

!macro TIDYCRAFT_EMIT_PATH_SCRIPT
  ; `$$` is a literal dollar for PowerShell; `$\"` a literal quote; `$\r$\n` CRLF.
  FileOpen $9 "$PLUGINSDIR\tidycraft-path.ps1" w
  FileWrite $9 "param([Parameter(Mandatory=$$true)][string]$$Dir,[switch]$$Remove)$\r$\n"
  FileWrite $9 "$$ErrorActionPreference='Stop'$\r$\n"
  FileWrite $9 "$$key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment',$$true)$\r$\n"
  FileWrite $9 "if($$null -eq $$key){exit 0}$\r$\n"
  FileWrite $9 "$$raw=[string]$$key.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)$\r$\n"
  FileWrite $9 "$$kept=@($$raw -split ';' | Where-Object { $$_ -ne '' -and $$_.TrimEnd('\') -ne $$Dir.TrimEnd('\') })$\r$\n"
  FileWrite $9 "if(-not $$Remove){$$kept+=$$Dir}$\r$\n"
  FileWrite $9 "$$new=($$kept -join ';')$\r$\n"
  FileWrite $9 "if($$new -ne $$raw){$$key.SetValue('Path',$$new,[Microsoft.Win32.RegistryValueKind]::ExpandString)}$\r$\n"
  FileWrite $9 "$$key.Close()$\r$\n"
  FileClose $9
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering the tidycraft command line tool..."
  !insertmacro TIDYCRAFT_EMIT_PATH_SCRIPT
  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\tidycraft-path.ps1" -Dir "$INSTDIR"'
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "Removing the tidycraft command line tool from PATH..."
  !insertmacro TIDYCRAFT_EMIT_PATH_SCRIPT
  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\tidycraft-path.ps1" -Dir "$INSTDIR" -Remove'
  Pop $0
!macroend
