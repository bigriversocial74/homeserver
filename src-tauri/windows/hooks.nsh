!include "LogicLib.nsh"

!define MG_SERVICE_NAME "MicrogifterHomeServer"
!define MG_SERVICE_DISPLAY "Microgifter HomeServer"
!define MG_INSTALL_LOG "$COMMONAPPDATA\Microgifter\HomeServer\logs\microgifter-homeserver-service.log-installer"

!macro MG_INSTALL_LOG MESSAGE
  CreateDirectory "$COMMONAPPDATA\Microgifter\HomeServer\logs"
  FileOpen $1 "${MG_INSTALL_LOG}" a
  FileWrite $1 "${MESSAGE}$\r$\n"
  FileClose $1
!macroend

!macro MG_REQUIRE_SUCCESS MESSAGE
  Pop $0
  ${If} $0 != 0
    DetailPrint "${MESSAGE} (exit code $0)."
    FileOpen $1 "${MG_INSTALL_LOG}" a
    FileWrite $1 "${MESSAGE} (exit code $0).$\r$\n"
    FileClose $1
    SetErrorLevel $0
    Quit
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
  CreateDirectory "$COMMONAPPDATA\Microgifter\HomeServer\logs"
  Delete "${MG_INSTALL_LOG}"
  !insertmacro MG_INSTALL_LOG "Starting HomeServer installer security and service registration"

  ; Protect the root first. Keeping this separate from the grant operation is
  ; required because a combined recursive icacls invocation can leave the root
  ; ACL inheriting from ProgramData on some Windows builds.
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /inheritance:r'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to disable inherited HomeServer data permissions"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to grant restricted HomeServer data permissions"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /setowner "*S-1-5-18"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to set the HomeServer data directory owner"

  ; Harden retained data during upgrades as well as the root directory itself.
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /inheritance:r /T /C'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to remove inherited permissions from retained HomeServer data"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F" /T /C'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to apply restricted permissions to retained HomeServer data"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONAPPDATA\Microgifter\HomeServer" /setowner "*S-1-5-18" /T /C'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to set retained HomeServer data ownership"

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

  !insertmacro MG_INSTALL_LOG "HomeServer installer security and service registration completed successfully"
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
