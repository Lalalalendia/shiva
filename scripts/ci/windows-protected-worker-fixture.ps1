# CI-only protected-root feasibility experiment. NEVER run on a workstation.
# This proves a proposed LPAC ACL shape on a disposable hosted runner,
# not that Akula's per-user NSIS release has a trusted immutable image root.
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'Protected worker ACL experiment is restricted to hosted Windows CI'
}
if (!$env:ProgramFiles -or !$env:GITHUB_RUN_ID -or !$env:SHIVA_TEST_PROFILE_SID) {
  throw 'Missing CI-only Program Files, run ID or exact AppContainer SID'
}
$sid = $env:SHIVA_TEST_PROFILE_SID
if ($sid -notmatch '^S-1-15-2-') { throw 'Unexpected worker AppContainer SID' }

function Checked-Icacls {
  param([string[]]$Args)
  & icacls.exe @Args | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw "icacls operation failed, exit=$LASTEXITCODE"
  }
}
function Assert-Narrow-Acl {
  param([string]$Path)
  $acl = Get-Acl -LiteralPath $Path
  if (!$acl.AreAccessRulesProtected) {
    throw 'Fixture must not inherit unreviewed DACL ACEs'
  }
  # The controlled test fixture permits System/Admins plus exact LPAC SID.
  # This is a structural DACL inspection, NOT an effective non-admin token
  # negative. Machine-wide installer and non-admin E2E remain release gates.
  $trusted = @('S-1-5-18', 'S-1-5-32-544', $sid)
  foreach ($ace in $acl.Access) {
    if ($ace.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) {
      continue
    }
    $principalSid = $ace.IdentityReference.Translate(
      [System.Security.Principal.SecurityIdentifier]).Value
    if ($trusted -notcontains $principalSid) {
      throw 'Unexpected allow ACE in protected worker CI fixture'
    }
    if ($principalSid -eq $sid) {
      $rights = [int]$ace.FileSystemRights
      $unsafe = [int]([System.Security.AccessControl.FileSystemRights]::WriteData) -bor
        [int]([System.Security.AccessControl.FileSystemRights]::AppendData) -bor
        [int]([System.Security.AccessControl.FileSystemRights]::Delete) -bor
        [int]([System.Security.AccessControl.FileSystemRights]::ChangePermissions) -bor
        [int]([System.Security.AccessControl.FileSystemRights]::TakeOwnership)
      if (($rights -band $unsafe) -ne 0) {
        throw 'Exact worker SID unexpectedly has mutation rights'
      }
    }
  }
}

$source = Join-Path $env:GITHUB_WORKSPACE 'target\debug\shiva-worker.exe'
if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
  throw 'Built Shiva worker missing from expected CI path'
}
$targetRoot = Join-Path $env:ProgramFiles ("AkulaShivaWorker-CI-{0}-{1}" -f
  $env:GITHUB_RUN_ID, $env:GITHUB_RUN_ATTEMPT)
if (Test-Path -LiteralPath $targetRoot) {
  throw 'Refusing to reuse an existing protected worker test directory'
}
$targetExe = Join-Path $targetRoot 'shiva-worker.exe'
$systemGrant = '*S-1-5-18:(OI)(CI)(F)'
$adminGrant = '*S-1-5-32-544:(OI)(CI)(F)'
$packageGrant = "*$($sid):(OI)(CI)(RX)"

try {
  New-Item -ItemType Directory -Path $targetRoot -ErrorAction Stop | Out-Null
  Copy-Item -LiteralPath $source -Destination $targetExe -ErrorAction Stop
  Checked-Icacls @($targetRoot, '/inheritance:r')
  Checked-Icacls @($targetRoot, '/grant:r', $systemGrant, $adminGrant, $packageGrant)
  Checked-Icacls @($targetExe, '/inheritance:r')
  Checked-Icacls @($targetExe, '/grant:r',
    '*S-1-5-18:(F)', '*S-1-5-32-544:(F)', "*$($sid):(RX)")
  Assert-Narrow-Acl $targetRoot
  Assert-Narrow-Acl $targetExe
  $originalHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
  $installedHash = (Get-FileHash -LiteralPath $targetExe -Algorithm SHA256).Hash
  if ($originalHash -ne $installedHash) {
    throw 'Protected fixture binary differs from built worker'
  }
  # Exercise exactly the same real subprocess, negative LPAC and nested Job
  # proof as on the ordinary CI build, but launch from the protected root.
  $env:SHIVA_TEST_WORKER_BIN = $targetExe
  cargo test -p shiva-worker-client --test host_process -- --nocapture
  if ($LASTEXITCODE -ne 0) { throw 'Protected-root AppContainer E2E failed' }
  cargo test -p shiva-worker-client --test worker_image_path -- --nocapture
  if ($LASTEXITCODE -ne 0) { throw 'Protected-root reparse/path E2E failed' }
  Write-Host 'SHIVA_WIN_PROTECTED_ROOT_CI=pass'
} finally {
  # This script only ever creates this exact unique test path on an ephemeral
  # GH runner. Failures cannot modify the installed user's real Akula.
  if (Test-Path -LiteralPath $targetRoot) {
    Remove-Item -LiteralPath $targetRoot -Recurse -Force -ErrorAction Stop
  }
}
