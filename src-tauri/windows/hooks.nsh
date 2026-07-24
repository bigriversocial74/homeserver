!define MG_SERVICE_NAME "MicrogifterHomeServer"
!define MG_SERVICE_DISPLAY "Microgifter HomeServer"

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering Microgifter HomeServer service"
  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop "${MG_SERVICE_NAME}"'
  nsExec::ExecToLog '"$SYSDIR\sc.exe" delete "${MG_SERVICE_NAME}"'
  nsExec::ExecToLog '"$SYSDIR\sc.exe" create "${MG_SERVICE_NAME}" binPath= "\"$INSTDIR\resources\microgifter-homeserver-service.exe\" service" start= auto DisplayName= "${MG_SERVICE_DISPLAY}"'
  nsExec::ExecToLog '"$SYSDIR\sc.exe" description "${MG_SERVICE_NAME}" "Private local Microgifter HomeServer services"'
  nsExec::ExecToLog '"$SYSDIR\sc.exe" failure "${MG_SERVICE_NAME}" reset= 86400 actions= restart/5000/restart/15000/none/0'
  nsExec::ExecToLog '"$SYSDIR\sc.exe" start "${MG_SERVICE_NAME}"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping Microgifter HomeServer service"
  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop "${MG_SERVICE_NAME}"'
  Sleep 1500
  nsExec::ExecToLog '"$SYSDIR\sc.exe" delete "${MG_SERVICE_NAME}"'
!macroend
