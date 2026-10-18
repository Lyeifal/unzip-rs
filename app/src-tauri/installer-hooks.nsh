; ProductName also controls NSIS's uninstall registry key. Retire a verified
; legacy installation so renaming the product does not leave two installed apps.
; /S preserves app data; the new application migrates it on first launch.
!macro NSIS_HOOK_PREINSTALL
  Push $R0
  Push $R1
  Push $R2
  ReadRegStr $R0 HKCU "Software\unzip\自动解压工具" ""
  ReadRegStr $R1 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\自动解压工具" "MainBinaryName"
  ${If} $R0 != ""
  ${AndIf} $R1 == "unzip-app.exe"
  ${AndIf} ${FileExists} "$R0\unzip-app.exe"
  ${AndIf} ${FileExists} "$R0\uninstall.exe"
    ClearErrors
    ExecWait '"$R0\uninstall.exe" /S _?=$R0' $R2
    ${If} ${Errors}
    ${OrIf} $R2 != 0
      MessageBox MB_OK|MB_ICONSTOP "Please close the legacy app and uninstall it before installing unzip-rs. Your application data will be retained."
      Pop $R2
      Pop $R1
      Pop $R0
      Abort
    ${EndIf}
    Delete "$R0\uninstall.exe"
    ; Only remove the old install directory when empty, never recursively.
    RMDir "$R0"
    DeleteRegKey HKCU "Software\unzip\自动解压工具"
  ${EndIf}
  Pop $R2
  Pop $R1
  Pop $R0
!macroend
