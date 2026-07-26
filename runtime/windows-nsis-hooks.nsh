; Tauri uses the MUI "Show readme" checkbox as "Create desktop shortcut".
; Override only the finish-page insertion macro and remove that option before
; MUI creates the page. Keep the normal "Run DictatingMe" finish-page option.
!macroundef MUI_PAGE_FINISH
!macro MUI_PAGE_FINISH
  !undef MUI_FINISHPAGE_SHOWREADME
  !undef MUI_FINISHPAGE_SHOWREADME_TEXT
  !undef MUI_FINISHPAGE_SHOWREADME_FUNCTION

  !verbose push
  !verbose ${MUI_VERBOSE}
  !insertmacro MUI_PAGE_INIT
  !insertmacro MUI_PAGEDECLARATION_FINISH
  !verbose pop
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; v0.2.0 and earlier used current-user installation under LocalAppData.
  ; Protect user data while removing that application copy once.
  IfFileExists "$LOCALAPPDATA\DictatingMe\uninstall.exe" 0 legacy_install_done
    RMDir /r "$TEMP\DictatingMe-user-data-migration"
    IfFileExists "$LOCALAPPDATA\com.dictatingme.app\*.*" 0 legacy_data_protected
      Rename "$LOCALAPPDATA\com.dictatingme.app" "$TEMP\DictatingMe-user-data-migration"
    legacy_data_protected:
    DetailPrint "Removing legacy current-user DictatingMe installation..."
    ExecWait '"$LOCALAPPDATA\DictatingMe\uninstall.exe" /S' $0
    DetailPrint "Legacy uninstaller exit code: $0"
    RMDir /r "$LOCALAPPDATA\DictatingMe"
    IfFileExists "$TEMP\DictatingMe-user-data-migration\*.*" 0 legacy_install_done
      RMDir /r "$LOCALAPPDATA\com.dictatingme.app"
      Rename "$TEMP\DictatingMe-user-data-migration" "$LOCALAPPDATA\com.dictatingme.app"
  legacy_install_done:
  RMDir /r "$INSTDIR\assets"
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Silent/passive mode creates a desktop shortcut in Tauri's default template.
  ; Remove it as DictatingMe only uses Start Menu and tray entry points.
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
!macroend
