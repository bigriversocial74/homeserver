!include "LogicLib.nsh"

!define MG_SERVICE_NAME "MicrogifterHomeServer"
!define MG_SERVICE_DISPLAY "Microgifter HomeServer"
!define MG_INSTALL_LOG_PATH "$COMMONPROGRAMDATA\Microgifter\HomeServer\logs\microgifter-homeserver-service.log-installer"
!define MG_TEMP_INSTALL_LOG_PATH "$TEMP\Microgifter-HomeServer-install.log"

!macro MG_WRITE_INSTALL_LOG MESSAGE
  CreateDirectory "$COMMONPROGRAMDATA\Microgifter\HomeServer\logs"
  FileOpen $1 "${MG_INSTALL_LOG_PATH}" a
  FileWrite $1 "${MESSAGE}$\r$\n"
  FileClose $1
  FileOpen $2 "${MG_TEMP_INSTALL_LOG_PATH}" a
  FileWrite $2 "${MESSAGE}$\r$\n"
  FileClose $2
!macroend

!macro MG_REQUIRE_SUCCESS MESSAGE
  Pop $0
  ${If} $0 != 0
    DetailPrint "${MESSAGE} (exit code $0)."
    FileOpen $1 "${MG_INSTALL_LOG_PATH}" a
    FileWrite $1 "${MESSAGE} (exit code $0).$\r$\n"
    FileClose $1
    FileOpen $2 "${MG_TEMP_INSTALL_LOG_PATH}" a
    FileWrite $2 "${MESSAGE} (exit code $0).$\r$\n"
    FileClose $2
    SetErrorLevel $0
    Quit
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Stopping Microgifter HomeServer service for installation"

  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop "${MG_SERVICE_NAME}"'
  Pop $0
  Sleep 1500

  DetailPrint "Hardening Microgifter HomeServer data permissions"
  CreateDirectory "$COMMONPROGRAMDATA\Microgifter\HomeServer"
  CreateDirectory "$COMMONPROGRAMDATA\Microgifter\HomeServer\logs"
  Delete "${MG_INSTALL_LOG_PATH}"
  Delete "${MG_TEMP_INSTALL_LOG_PATH}"
  !insertmacro MG_WRITE_INSTALL_LOG "Starting HomeServer installer security and service registration"

  ; Establish explicit SYSTEM and Administrators access before removing any
  ; inherited ACEs. Removing inheritance first can make retained databases and
  ; logs inaccessible between icacls commands during an upgrade.
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONPROGRAMDATA\Microgifter\HomeServer" /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to grant restricted HomeServer data permissions"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONPROGRAMDATA\Microgifter\HomeServer" /setowner "*S-1-5-18"'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to set the HomeServer data directory owner"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONPROGRAMDATA\Microgifter\HomeServer" /inheritance:r'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to disable inherited HomeServer data permissions"

  ; Reset every retained child to inherit only the protected root ACL. This
  ; removes stale explicit access without creating a no-access interval. Do not
  ; continue on ACL failures: an update must abort rather than leave data files
  ; unreadable to the LocalSystem service.
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONPROGRAMDATA\Microgifter\HomeServer\*" /reset /T'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to reset retained HomeServer data permissions"

  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$COMMONPROGRAMDATA\Microgifter\HomeServer\*" /setowner "*S-1-5-18" /T'
  !insertmacro MG_REQUIRE_SUCCESS "Unable to set retained HomeServer data ownership"

  ; Keep the existing service registration during upgrades. Deleting and
  ; recreating it can leave the Service Control Manager in a pending-deletion
  ; state while an automatic rollback is trying to restore the prior release.
  nsExec::ExecToLog '"$SYSDIR\sc.exe" query "${MG_SERVICE_NAME}"'
  Pop $0
  ${If} $0 == 0
    nsExec::ExecToLog '"$SYSDIR\sc.exe" config "${MG_SERVICE_NAME}" binPath= "\"$INSTDIR\resources\microgifter-homeserver-service.exe\" service" start= auto DisplayName= "${MG_SERVICE_DISPLAY}"'
    !insertmacro MG_REQUIRE_SUCCESS "Unable to update the Microgifter HomeServer service registration"
  ${Else}
    nsExec::ExecToLog '"$SYSDIR\sc.exe" create "${MG_SERVICE_NAME}" binPath= "\"$INSTDIR\resources\microgifter-homeserver-service.exe\" service" start= auto DisplayName= "${MG_SERVICE_DISPLAY}"'
    !insertmacro MG_REQUIRE_SUCCESS "Unable to register the Microgifter HomeServer service"
  ${EndIf}

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

  !insertmacro MG_WRITE_INSTALL_LOG "HomeServer installer security and service registration completed successfully"
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
