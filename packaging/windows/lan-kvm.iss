#define RepoRoot ExtractFileDir(ExtractFileDir(RemoveBackslash(SourcePath)))
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe RepoRoot + "\target\release\lan-kvm.exe"
#endif
#ifndef OutputDir
  #define OutputDir RepoRoot + "\target\installer"
#endif

[Setup]
AppId={{E0136334-9019-40D0-8ACB-44E7A03C0F2D}
AppName=Lan Kvm
AppVersion={#AppVersion}
AppVerName=Lan Kvm {#AppVersion}
AppPublisher=Gabriel Griffo
VersionInfoCompany=Gabriel Griffo
VersionInfoDescription=Instalador do Lan Kvm
VersionInfoProductName=Lan Kvm
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Lan Kvm
UsePreviousAppDir=no
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=lan-kvm-{#AppVersion}-windows-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile={#RepoRoot}\branding\lan-kvm.ico
CloseApplications=no
UninstallDisplayIcon={app}\Lan Kvm.exe
UninstallDisplayName=Lan Kvm

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Lan Kvm.exe"; Flags: ignoreversion

[InstallDelete]
Type: filesandordirs; Name: "{localappdata}\Programs\lan-kvm"
Type: files; Name: "{userprograms}\lan-kvm.lnk"

[Icons]
Name: "{userprograms}\Lan Kvm"; Filename: "{app}\Lan Kvm.exe"; Comment: "Um teclado e um mouse para o PC e o notebook"

[Run]
Filename: "{app}\Lan Kvm.exe"; Description: "Abrir o Lan Kvm"; Flags: nowait postinstall

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValue = 'Lan Kvm';
  OldRunValue = 'lan-kvm';

procedure StopLanKvm;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM lan-kvm.exe /IM "Lan Kvm.exe"', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(500);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopLanKvm;
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Command: String;
begin
  if (CurStep = ssPostInstall) and RegQueryStringValue(HKCU, RunKey, OldRunValue, Command) then
  begin
    RegWriteStringValue(HKCU, RunKey, RunValue, '"' + ExpandConstant('{app}\Lan Kvm.exe') + '" --tray');
    RegDeleteValue(HKCU, RunKey, OldRunValue);
  end;
end;

function InitializeUninstall: Boolean;
begin
  StopLanKvm;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
begin
  if (CurUninstallStep = usPostUninstall)
    and RegQueryStringValue(HKCU, RunKey, RunValue, Command)
    and (Pos(Lowercase(ExpandConstant('{app}')), Lowercase(Command)) > 0) then
    RegDeleteValue(HKCU, RunKey, RunValue);
end;
