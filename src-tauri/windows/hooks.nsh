!include "LogicLib.nsh"

!define MG_SERVICE_NAME "MicrogifterHomeServer"
!define MG_SERVICE_DISPLAY "Microgifter HomeServer"

!macro MG_REQUIRE_SUCCESS MESSAGE
  Pop $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "${MESSAGE} (exit code $0)."
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering Microgifter HomeServer service"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop "${MG_SERVICE_NAME}"'
  Pop $0
  nsExec::ExecToLog '"$SYSDIR\sc.exe" delete "${MG_SERVICE_NAME}"'
  Pop $0
  Sleep 1500

  DetailPrint "Hardening Microgifter HomeServer data permissions"
  CreateDirectory "$COMMONAPPDATA\Microgifter\HomeServer"
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /inheritance:r /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F" /T /C'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to secure the HomeServer data directory"
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /setowner "*S-1-5-18" /T /C'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to set the HomeServer data directory owner"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" create "${MG_SERVICE_NAME}" binPath= "\"$INSTDIR\resources\microgifter-homeserver-service.exe\" service" start= auto DisplayName= "${MG_SERVICE_DISPLAY}"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to register the Microgifter HomeServer service"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" config "${MG_SERVICE_NAME}" start= delayed-auto'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to configure delayed automatic startup"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" description "${MG_SERVICE_NAME}" "Private local Microgifter HomeServer services"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to set the HomeServer service description"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" failure "${MG_SERVICE_NAME}" reset= 86400 actions= restart/5000/restart/15000/none/0'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to configure HomeServer service recovery"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" failureflag "${MG_SERVICE_NAME}" 1'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to enable HomeServer failure actions"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" sidtype "${MG_SERVICE_NAME}" unrestricted'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to configure the HomeServer service identity"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" start "${MG_SERVICE_NAME}"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to start the Microgifter HomeServer service"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping Microgifter HomeServer service"
  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop "${MG_SERVICE_NAME}"'
  Pop $0
  Sleep 2000
  nsExec::ExecToLog '"$SYSDIR\sc.exe" delete "${MG_SERVICE_NAME}"'
  Pop $0
  Sleep 1000
!macroend
