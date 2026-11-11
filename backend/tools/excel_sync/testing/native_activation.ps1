param(
    [Parameter(Mandatory)]$Workbook,
    [Parameter(Mandatory)][string]$FixturePath,
    [Parameter(Mandatory)][string]$TestRoot
)
# Per-computer activation in native Excel against the isolated loopback fixture.
# The caller owns Excel (started through automation, so UserControl is false and
# no password dialog can block) and opens a fresh empty test copy in TestRoot.
# A computer "without activation" is simulated by deleting this copy's WinCred
# entry, which is exactly the state of the same file on another computer.
# The caller must take the reopened copy from Workbooks afterwards.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem

$app = $Workbook.Application
$bookPath = [string]$Workbook.FullName
$views = @('CardXabar', 'HUMO Card', [regex]::Unescape('\u0412\u0441\u0435 \u0442\u0440\u0430\u043d\u0437\u0430\u043a\u0446\u0438\u0438'))
$lockName = [regex]::Unescape('\u0414\u043e\u0441\u0442\u0443\u043f')
$wrongPassword = [regex]::Unescape('\u041d\u0435\u0432\u0435\u0440\u043d\u044b\u0439 \u043f\u0430\u0440\u043e\u043b\u044c.')
$notActivated = [regex]::Unescape('\u043d\u0435 \u0430\u043a\u0442\u0438\u0432\u0438\u0440\u043e\u0432\u0430\u043d')
$noPassword = [regex]::Unescape('\u043d\u0435 \u0437\u0430\u0434\u0430\u043b \u043f\u0430\u0440\u043e\u043b\u044c')
$revoked = [regex]::Unescape('\u043e\u0442\u043e\u0437\u0432\u0430\u043d')
$password = 'synthetic-' + [guid]::NewGuid().ToString('N').Substring(0, 12)
$reportPath = Join-Path $TestRoot 'native-activation-verification.json'
$report = [ordered]@{native=$true; passed=$false; phase='prepare'; checks=@(); workbook_path=$bookPath}
$session = $null

function Assert-That([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Pass([string]$Name) {
    $script:report.checks += $Name
    $script:report.phase = $Name
    $script:report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $script:reportPath -Encoding UTF8
}
function Server { return Invoke-ExcelSyncFixtureControl $script:session GET '/__test__/state' }
function Owner-Api([string]$Method, [string]$Path, $Body) {
    # The fixture overrides the dashboard login with synthetic owner 1.
    $request = @{Uri=([string]$script:session.api_url + $Path); Method=$Method; TimeoutSec=20; UseBasicParsing=$true}
    if ($null -ne $Body) {
        $request.ContentType = 'application/json; charset=utf-8'
        $request.Body = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Compress))
    }
    return Invoke-WebRequest @request
}
function Macro([string]$Name, [object[]]$Arguments = @()) { return Invoke-ExcelSyncMacro $script:Workbook $Name $Arguments }
function Visible-Sheets {
    return @($script:Workbook.Worksheets | Where-Object { $_.Visible -eq -1 } | ForEach-Object { [string]$_.Name })
}
function Assert-Locked([string]$Context) {
    $shown = @(Visible-Sheets)
    Assert-That (($shown.Count -eq 1) -and ($shown[0] -ceq $script:lockName)) "$Context`: only the lock sheet may be visible, got $($shown -join ', ')"
    foreach ($name in $script:views) {
        Assert-That ($script:Workbook.Worksheets.Item($name).Visible -eq 2) "$Context`: $name must be veryHidden"
    }
    Assert-That (-not [bool](Macro 'ExcelSync.IsActivated')) "$Context`: the client must not be activated"
}
function Assert-Revealed([string]$Context) {
    foreach ($name in $script:views) {
        Assert-That ($script:Workbook.Worksheets.Item($name).Visible -eq -1) "$Context`: $name must be visible"
    }
    Assert-That ($script:Workbook.Worksheets.Item($script:lockName).Visible -eq 2) "$Context`: the lock sheet must be hidden"
    Assert-That ([bool](Macro 'ExcelSync.IsActivated')) "$Context`: the client must be activated"
}
function Saved-States {
    # Read the file as Excel wrote it; Excel keeps its own handle open.
    $stream = [IO.File]::Open($script:bookPath, 'Open', 'Read', 'ReadWrite')
    try {
        $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read)
        $reader = [IO.StreamReader]::new($zip.GetEntry('xl/workbook.xml').Open(), [Text.Encoding]::UTF8)
        [xml]$xml = $reader.ReadToEnd()
        $reader.Dispose(); $zip.Dispose()
    } finally { $stream.Dispose() }
    $states = @{}
    foreach ($sheet in $xml.workbook.sheets.sheet) {
        $state = [string]$sheet.GetAttribute('state')
        if ($state.Length -eq 0) { $state = 'visible' }
        $states[[string]$sheet.name] = $state
    }
    return $states
}
function Assert-SavedLocked([string]$Context) {
    $states = Saved-States
    Assert-That ($states[$script:lockName] -ceq 'visible') "$Context`: the saved lock sheet must be visible"
    foreach ($name in $script:views) {
        Assert-That ($states[$name] -ceq 'veryHidden') "$Context`: saved $name must be veryHidden, got $($states[$name])"
    }
}
function Save-WithEvents {
    # Save the way a user does (Ctrl+S). A COM Workbook.Save() silently drops
    # the save that the client repeats from inside BeforeSave.
    $old = $script:app.EnableEvents
    try {
        $script:app.EnableEvents = $true
        $script:Workbook.Activate()
        $script:app.CommandBars.ExecuteMso('FileSave')
    } finally { $script:app.EnableEvents = $old }
}
function Reopen {
    # A COM Close cannot run the client's own save from BeforeClose (see
    # EXCEL_SYNC.md); the file is already saved, so stop the client first.
    [void](Macro 'ExcelSync.StopSync')
    $script:Workbook.Close($false)
    $script:app.EnableEvents = $true
    $script:Workbook = $script:app.Workbooks.Open($script:bookPath)
}
function Credential { return [string](Macro 'SyncWindows.ReadCredential' @($script:session.workbook_id)) }
function Active-DeviceGrants {
    return @((Server).device_grants | Where-Object { [string]$_.workbook_id -ceq $script:session.workbook_id -and -not $_.revoked })
}
function Requests-Since([int]$Start) { return @((Server).requests | Select-Object -Skip $Start | Where-Object { $_.path -like '/api/excel/v1/*' }) }

try {
    $fixture = Get-Content -LiteralPath $FixturePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $session = Initialize-ExcelSyncTestClient -Workbook $Workbook -FixturePath $FixturePath -TestRoot $TestRoot
    Assert-Revealed 'unshared owner copy'

    # 1. The owner's first sync trades the older connection key for a key that
    #    works only on this computer; the older key itself stays valid.
    $state = Read-ExcelSyncTestState $session
    Assert-That ([bool]$state.device_bound) 'The first sync must bind the connection to this computer.'
    $grants = Active-DeviceGrants
    Assert-That ($grants.Count -eq 1) 'Expected exactly one live per-computer grant.'
    Assert-That ([string]$grants[0].device_id -match '^(mg|hw):[0-9a-f-]{8,40}:[0-9a-f]+$') 'Unexpected device identity format.'
    Assert-That (-not ((Server).grants | Where-Object { [string]$_.id -ceq [string]$fixture.grant_id }).revoked) 'The older shared grant must stay valid.'
    $current = Credential
    Assert-That ($current.StartsWith('excel_') -and $current -cne [string]$fixture.token) 'WinCred must hold the new per-computer key.'
    $current = $null
    Assert-That (@((Read-ExcelSyncTestState $session).records.PSObject.Properties).Count -ge 2) 'The owner copy must load the receipts.'
    Pass 'legacy_key_bound_to_device'

    # 2. The owner protects the workbook and distributes it as a shared copy.
    [void](Owner-Api PUT '/api/excel/activations' @{workbook_id=$session.workbook_id; password=$password})
    [void](Macro 'ExcelSync.StopSync')
    $app.EnableEvents = $false
    $config = $Workbook.Worksheets.Item('_sync')
    $config.Unprotect('tbs-layout')
    $config.Range('B5').Value2 = [string]$session.api_url
    $config.Range('B6').Value2 = 'excel_synthetic_stale_shared_key_must_be_dropped'
    $config.Range('B7').Value2 = [string]$session.owner_id
    $config.Range('B8').Value2 = '1'
    $config.Protect('tbs-layout')
    Save-WithEvents
    Assert-SavedLocked 'saved without a running client'
    Assert-Revealed 'after saving on the activated computer'
    Pass 'saved_file_hides_receipts'

    # 3. The same file on a computer without activation.
    [void](Macro 'SyncWindows.DeleteCredential' @($session.workbook_id))
    Assert-That (-not (Test-ExcelSyncTestCredentialExists $session.workbook_id)) 'The simulated new computer must have no key.'
    $before = @((Server).requests).Count
    Reopen
    Assert-Locked 'new computer'
    Assert-That ([string]$Workbook.Worksheets.Item('_sync').Range('B6').Value2 -ceq '') 'The stale shared key must be removed from the file.'
    Assert-That ([string]$Workbook.Worksheets.Item($lockName).Range('A6').Value2).Contains($notActivated) 'The lock sheet must explain the activation.'
    [void](Macro 'ExcelSync.SyncNow')
    Assert-That ((Requests-Since $before).Count -eq 0) 'A computer without activation must not contact the sync API.'
    Assert-That (-not (Test-ExcelSyncTestCredentialExists $session.workbook_id)) 'No key may be created before activation.'
    Pass 'new_computer_sees_only_lock_sheet'

    # 4. Wrong password, removed password, then the right one.
    $answer = [string](Macro 'ExcelSync.ActivateWithPassword' @('wrong-password'))
    Assert-That ($answer -ceq $wrongPassword) "Unexpected wrong-password answer: $answer"
    Assert-Locked 'after a wrong password'
    [void](Owner-Api DELETE ('/api/excel/activations/' + $session.workbook_id) $null)
    $answer = [string](Macro 'ExcelSync.ActivateWithPassword' @($password))
    Assert-That ($answer.Contains($noPassword)) "Unexpected answer without a password: $answer"
    Assert-Locked 'after the owner removed the password'
    [void](Owner-Api PUT '/api/excel/activations' @{workbook_id=$session.workbook_id; password=$password})
    $answer = [string](Macro 'ExcelSync.ActivateWithPassword' @($password))
    Assert-That ($answer.Length -eq 0) "Activation failed: $answer"
    Assert-Revealed 'after activation'
    Assert-That (Test-ExcelSyncTestCredentialExists $session.workbook_id) 'Activation must store this computer''s key.'
    Assert-That ((Active-DeviceGrants).Count -eq 1) 'A repeated activation of one computer must replace its grant.'
    [void](Invoke-ExcelSyncTestSync $Workbook $session)
    $server = Server
    Assert-That (@((Read-ExcelSyncTestState $session).records.PSObject.Properties).Count -eq @($server.records).Count) 'All receipts must load after activation.'
    Pass 'password_activates_and_loads'

    # 5. Saving keeps the file locked; this computer is never asked again.
    Save-WithEvents
    Assert-That ([bool]$Workbook.Saved) 'Revealing after the save must not dirty the workbook.'
    Assert-SavedLocked 'saved on the activated computer'
    Assert-Revealed 'after saving'
    $before = @((Server).requests).Count
    Reopen
    Assert-Revealed 'reopened on the activated computer'
    [void](Invoke-ExcelSyncTestSync $Workbook $session)
    Assert-That ((Requests-Since $before | Where-Object { $_.path -eq '/api/excel/v1/activate' }).Count -eq 0) 'An activated computer must not activate again.'
    Pass 'activated_computer_is_remembered'

    # 6. The owner revokes this computer: the receipts lock again.
    [void](Invoke-ExcelSyncFixtureControl $session POST '/__test__/revoke-devices' @{})
    [void](Macro 'ExcelSync.SyncNow')
    Assert-Locked 'after revocation'
    Assert-That (-not (Test-ExcelSyncTestCredentialExists $session.workbook_id)) 'A revoked key must be removed from WinCred.'
    Assert-That ([string]$Workbook.Worksheets.Item($lockName).Range('A6').Value2).Contains($revoked) 'The lock sheet must say the access was revoked.'
    Pass 'revoked_computer_locks_again'

    $report.passed = $true
    $report.phase = 'complete'
} catch {
    $report.error = $_.Exception.Message
    $report.stack = $_.ScriptStackTrace
    throw
} finally {
    $password = $null
    try { [void](Macro 'ExcelSync.StopSync') } catch {}
    if ($null -ne $session) { try { [void](Macro 'SyncWindows.DeleteCredential' @($session.workbook_id)) } catch {} }
    $report.updated_utc = [datetime]::UtcNow.ToString('o')
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}
