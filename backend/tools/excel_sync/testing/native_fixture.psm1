# Isolated test helpers only. Never imported into a distributed workbook.
# The caller owns Excel and opens the explicit test copy before using this module.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-ExcelSyncMacro {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)][string]$Name,
          [object[]]$Arguments = @())
    $macro = "'" + ([string]$Workbook.Name).Replace("'", "''") + "'!" + $Name
    $app = $Workbook.Application
    if ($Arguments.Count -gt 2) { throw 'This fixture helper accepts at most two macro arguments.' }
    try {
        switch ($Arguments.Count) {
            0 { return $app.Run($macro) }
            1 { return $app.Run($macro, $Arguments[0]) }
            2 { return $app.Run($macro, $Arguments[0], $Arguments[1]) }
        }
    } catch {
        # PowerShell 7 cannot marshal the null BSTR of an empty VBA String result.
        $inner = $_.Exception
        while ($null -ne $inner -and -not ($inner -is [ArgumentNullException] -and $inner.ParamName -eq 'ptr')) { $inner = $inner.InnerException }
        if ($null -ne $inner) { return '' }
        throw
    }
}

function Assert-ExcelSyncTestCopy {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)][string]$TestRoot)
    $rootPath = [IO.Path]::GetFullPath($TestRoot).TrimEnd('\', '/')
    $bookPath = [IO.Path]::GetFullPath([string]$Workbook.FullName)
    if (-not $bookPath.StartsWith($rootPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The workbook must be an explicit test copy inside TestRoot.'
    }
    if ([IO.Path]::GetExtension($bookPath) -ne '.xlsm' -or $Workbook.ReadOnly) {
        throw 'Use a writable .xlsm test copy.'
    }
    return $bookPath
}

function Read-ExcelSyncTestState {
    param([Parameter(Mandatory)]$Session)
    $state = Get-Content -LiteralPath $Session.state_path -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$state.workbook_id -ne [string]$Session.workbook_id) { throw 'Test state ID mismatch.' }
    return $state
}

function Write-ExcelSyncTestState {
    param([Parameter(Mandatory)]$Session, [Parameter(Mandatory)]$State)
    if ([string]$State.workbook_id -ne [string]$Session.workbook_id) { throw 'Test state ID mismatch.' }
    $target = [IO.Path]::GetFullPath([string]$Session.state_path)
    $expected = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ('TBSparcer\ExcelSync\' + [string]$Session.workbook_id + '\state.json')
    if ($target -ne [IO.Path]::GetFullPath($expected)) { throw 'Unexpected local test state path.' }
    $json = $State | ConvertTo-Json -Depth 100
    [IO.File]::WriteAllText($target, $json, [Text.UTF8Encoding]::new($false))
}

function Invoke-ExcelSyncFixtureControl {
    param([Parameter(Mandatory)]$Session, [Parameter(Mandatory)][ValidateSet('GET','POST','DELETE')][string]$Method,
          [Parameter(Mandatory)][string]$Path, $Body)
    if ($Session.api_url -ne 'http://127.0.0.1:8767' -or -not $Path.StartsWith('/__test__/')) {
        throw 'Fixture controls are restricted to the isolated loopback helper.'
    }
    $args = @{Uri=([string]$Session.api_url + $Path); Method=$Method; TimeoutSec=20}
    if ($null -ne $Body) {
        $args.ContentType = 'application/json; charset=utf-8'
        $json = $Body | ConvertTo-Json -Depth 30 -Compress
        $args.Body = [Text.Encoding]::UTF8.GetBytes($json)
    }
    # Windows PowerShell 5 can decode a JSON response without an explicit
    # charset as Latin-1. Inspect the actual UTF-8 wire bytes instead, so native
    # Unicode assertions exercise Excel/the API rather than that client bug.
    $response = Invoke-WebRequest @args -UseBasicParsing
    $bytes = $response.RawContentStream.ToArray()
    if ($bytes.Length -eq 0) { return $null }
    $json = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    return $json | ConvertFrom-Json
}

function Initialize-ExcelSyncTestClient {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)][string]$FixturePath,
          [Parameter(Mandatory)][string]$TestRoot)
    $bookPath = Assert-ExcelSyncTestCopy -Workbook $Workbook -TestRoot $TestRoot
    $fixture = Get-Content -LiteralPath $FixturePath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($fixture.api_url -ne 'http://127.0.0.1:8767') { throw 'Only the isolated fixture server is allowed.' }
    if ([string]::IsNullOrWhiteSpace([string]$fixture.token)) { throw 'Fixture grant is missing.' }
    $workbookId = [guid]::NewGuid().ToString()
    $directory = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ('TBSparcer\ExcelSync\' + $workbookId)
    if (Test-Path -LiteralPath $directory) { throw 'A fresh test ID unexpectedly exists.' }
    $session = [pscustomobject]@{
        test_marker='TBSparcer isolated native Excel test'; workbook_id=$workbookId;
        workbook_path=$bookPath; test_root=[IO.Path]::GetFullPath($TestRoot);
        state_path=(Join-Path $directory 'state.json'); api_url=[string]$fixture.api_url;
        owner_id=[string]$fixture.user_id; instance_id=[string]$fixture.instance_id;
        session_path=(Join-Path $TestRoot ('native-session-' + $workbookId + '.json'))
    }
    [void](New-Item -ItemType Directory -Path $directory)
    $state = [ordered]@{schema=1;workbook_id=$workbookId;records=@{};intents=@{};outbox=@{};blocked=@{};
                       server_url=[string]$fixture.api_url;paused=$false;owner_id=[string]$fixture.user_id}
    $app = $Workbook.Application
    $oldEvents = $app.EnableEvents
    try {
        $app.EnableEvents = $false
        [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync')
        $config = $Workbook.Worksheets.Item('_sync')
        $config.Unprotect('tbs-layout')
        $config.Range('B1').NumberFormat = '@'
        $config.Range('B1').Value2 = $workbookId
        $config.Protect('tbs-layout')
        $Workbook.Save()
        Write-ExcelSyncTestState $session $state
        # Exercise the actual VBA WinCred declarations; never print the token.
        [void](Invoke-ExcelSyncMacro $Workbook 'SyncWindows.SaveCredential' @($workbookId, [string]$fixture.token))
        $roundTrip = [string](Invoke-ExcelSyncMacro $Workbook 'SyncWindows.ReadCredential' @($workbookId))
        if ($roundTrip -cne [string]$fixture.token) { throw 'VBA WinCred round-trip failed.' }
        $roundTrip = $null
        [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StartSync')
        [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.SyncNow')
        $current = Read-ExcelSyncTestState $session
        if ([string]$current.owner_id -ne [string]$fixture.user_id) { throw 'Snapshot owner binding failed.' }
        $session | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $session.session_path -Encoding UTF8
        return $session
    } finally {
        $fixture.token = $null
        $app.EnableEvents = $oldEvents
    }
}

function Test-ExcelSyncTestCredentialExists {
    param([Parameter(Mandatory)][string]$WorkbookId)
    $parsedId = [guid]::Empty
    if (-not [guid]::TryParse($WorkbookId, [ref]$parsedId)) { throw 'Invalid synthetic workbook ID.' }
    $target = 'TBSparcer.ExcelSync.' + $WorkbookId
    $listing = (& (Join-Path $env:SystemRoot 'System32\cmdkey.exe') ('/list:' + $target) 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the synthetic credential target.' }
    # cmdkey echoes the requested target in the first nonempty heading.
    $details = (($listing -split '\r?\n' | Where-Object {$_.Trim().Length -gt 0} | Select-Object -Skip 1) -join "`n")
    return $details.IndexOf($target, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

function Initialize-ExcelSyncSharedTestProfile {
    # Prepare a shared LOOPBACK profile on a computer that is already activated:
    # the key lives only in WinCred, never in the file. Do not seed state: the
    # caller must reopen this saved copy to exercise Workbook_Open itself.
    # Password activation of a new computer is covered by native_activation.ps1.
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)][string]$FixturePath,
          [Parameter(Mandatory)][string]$TestRoot)
    $bookPath = Assert-ExcelSyncTestCopy $Workbook $TestRoot
    $fixture = Get-Content -LiteralPath $FixturePath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $fixture.synthetic -or $fixture.api_url -cne 'http://127.0.0.1:8767' -or [string]$fixture.user_id -cne '1') {
        throw 'Use the isolated loopback fixture and its synthetic owner 1.'
    }
    if ([string]::IsNullOrWhiteSpace([string]$fixture.token)) { throw 'Fixture grant is missing.' }
    $config = $Workbook.Worksheets.Item('_sync')
    $existingOrigin = [string]$config.Range('B5').Value2
    if ($existingOrigin.Length -gt 0 -and $existingOrigin -cne [string]$fixture.api_url) {
        throw 'The test copy contains a non-fixture connection.'
    }
    if ([string]$config.Range('A6').Value2 -cne 'shared_token' -or [string]$config.Range('A8').Value2 -cne 'shared_mode') {
        throw 'Build the shared workbook before running this harness.'
    }
    foreach ($sheet in $Workbook.Worksheets) {
        if ([string]$sheet.Name -ceq 'Control') { throw 'The shared smoke requires the workbook without Control.' }
    }
    $grid = $Workbook.Worksheets.Item([string]$config.Range('B3').Value2)
    if ($grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row -ge 5) {
        throw 'Use a fresh empty test copy, not an already populated workbook.'
    }
    $workbookId = [guid]::NewGuid().ToString()
    $directory = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ('TBSparcer\ExcelSync\' + $workbookId)
    if (Test-Path -LiteralPath $directory) { throw 'A fresh test profile unexpectedly exists.' }
    if (Test-ExcelSyncTestCredentialExists $workbookId) { throw 'The fresh test ID unexpectedly has a credential.' }
    $session = [pscustomobject]@{
        test_marker='TBSparcer isolated native Excel test'; shared_bootstrap=$true;
        workbook_id=$workbookId; workbook_path=$bookPath; test_root=[IO.Path]::GetFullPath($TestRoot);
        state_path=(Join-Path $directory 'state.json'); api_url=[string]$fixture.api_url;
        owner_id=[string]$fixture.user_id; instance_id=[string]$fixture.instance_id;
        grant_id=[string]$fixture.grant_id;
        session_path=(Join-Path $TestRoot ('native-shared-session-' + $workbookId + '.json'))
    }
    # Validate fixture identity and an unused grant before changing the copy.
    $server = Invoke-ExcelSyncFixtureControl $session GET '/__test__/state'
    if (-not $server.synthetic -or [string]$server.instance_id -cne $session.instance_id) { throw 'Fixture instance mismatch.' }
    $grant = @($server.grants | Where-Object {[string]$_.id -ceq $session.grant_id})
    if ($grant.Count -ne 1 -or $grant[0].revoked -or -not [string]::IsNullOrEmpty([string]$grant[0].workbook_id)) {
        throw 'Supply a new unbound fixture grant for this fresh workbook ID.'
    }
    $app = $Workbook.Application
    $oldEvents = $app.EnableEvents
    try {
        [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync')
        $app.EnableEvents = $false
        $config.Unprotect('tbs-layout')
        $config.Range('B1:B9').NumberFormat = '@'
        $config.Range('B1').Value2 = $workbookId
        $config.Range('B5').Value2 = [string]$fixture.api_url
        [void]$config.Range('B6').ClearContents()
        $config.Range('B7').Value2 = [string]$fixture.user_id
        $config.Range('B8').Value2 = '1'
        [void]$config.Range('B9').ClearContents()
        $config.Protect('tbs-layout')
        $Workbook.Save()
        if (Test-Path -LiteralPath $directory) { throw 'Profile setup unexpectedly seeded local state.' }
        # Exercise the actual VBA WinCred declarations; never print the token.
        [void](Invoke-ExcelSyncMacro $Workbook 'SyncWindows.SaveCredential' @($workbookId, [string]$fixture.token))
        if (-not (Test-ExcelSyncTestCredentialExists $workbookId)) { throw 'Profile setup could not activate this computer.' }
        $session | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $session.session_path -Encoding UTF8
        return $session
    } finally {
        $fixture.token = $null
        $app.EnableEvents = $oldEvents
    }
}

function Get-ExcelSyncTestStatus {
    param([Parameter(Mandatory)]$Workbook)
    foreach ($sheet in $Workbook.Worksheets) {
        if ([string]$sheet.Name -ceq 'Control') { return [string]$sheet.Range('B3').Value2 }
    }
    $name = [string]$Workbook.Worksheets.Item('_sync').Range('B3').Value2
    return [string]$Workbook.Worksheets.Item($name).Range('T1').Value2
}

function Get-ExcelSyncTestRow {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)][string]$Id)
    $name = [string]$Workbook.Worksheets.Item('_sync').Range('B3').Value2
    $sheet = $Workbook.Worksheets.Item($name)
    $last = $sheet.Cells.Item($sheet.Rows.Count, 52).End(-4162).Row
    for ($row=5; $row -le $last; $row++) {
        if ([string]$sheet.Cells.Item($row, 51).Value2 -ceq $Id) { return $row }
    }
    throw "The test receipt ID $Id is not visible in the workbook."
}

function Set-ExcelSyncTestFields {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session,
          [Parameter(Mandatory)][string]$Id, [Parameter(Mandatory)][hashtable]$Fields)
    $row = Get-ExcelSyncTestRow $Workbook $Id
    return Set-ExcelSyncTestRowFields -Workbook $Workbook -Session $Session -Row $row -Fields $Fields
}

function Set-ExcelSyncTestRowFields {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session,
          [Parameter(Mandatory)][int]$Row, [Parameter(Mandatory)][System.Collections.IDictionary]$Fields)
    [void](Assert-ExcelSyncTestCopy $Workbook $Session.test_root)
    if ($Row -lt 5) { throw 'Only data/input rows can be edited by the fixture helper.' }
    $mapping = @{transaction_date=17;transaction_type=20;operator_raw=21;application_mapped=23;amount=26;
                 balance_after=33;card_last_4=34;is_p2p=35;annotation_details=37;annotation_person_code=38;
                 annotation_period=39;annotation_article_code=40;currency=41;receiver_name=42;receiver_card=43;raw_message=44}
    $sheet = $Workbook.Worksheets.Item([string]$Workbook.Worksheets.Item('_sync').Range('B3').Value2)
    $app = $Workbook.Application
    $oldEvents = $app.EnableEvents
    try {
        # Use the real Workbook_SheetChange dispatch. No VBProject access or UI input.
        $app.EnableEvents = $true
        foreach ($field in $Fields.Keys) {
            if (-not $mapping.ContainsKey($field)) { throw "Unsupported editable field: $field" }
            $column = [int]$mapping[$field]
            $value = $Fields[$field]
            try {
                $cell = $sheet.Cells.Item($row, $column)
                if ($null -eq $value) {
                    # Explicit clear also raises the actual WorksheetChange event.
                    [void]$cell.ClearContents()
                } else {
                    switch ($field) {
                        {$_ -in @('transaction_date','annotation_period')} { $value=([datetime]$value).ToOADate() }
                        {$_ -in @('amount','balance_after')} { $value=[double]$value }
                        'is_p2p' { $value=[double][bool]$value }
                        default { $value=[string]$value }
                    }
                    # Windows PowerShell's cached COM property setter can retain
                    # the previous BSTR argument type. Dispatch an explicit boxed
                    # VARIANT through reflection for each string/numeric value.
                    $putArguments = [object[]]::new(1)
                    $putArguments[0] = $value
                    # Excel rejects invariant LCID 127 for this COM property put.
                    [void]$cell.GetType().InvokeMember('Value2', [Reflection.BindingFlags]::SetProperty,
                        $null, $cell, $putArguments, [Globalization.CultureInfo]::GetCultureInfo('en-US'))
                }
            } catch {
                $valueType = if ($null -eq $value) { 'null/clear' } else { $value.GetType().FullName }
                throw "Native test setter failed for field '$field', cell R${row}C${column}, value type ${valueType}: $($_.Exception.Message)"
            }
        }
    } finally { $app.EnableEvents = $oldEvents }
    return Read-ExcelSyncTestState $Session
}

function Set-ExcelSyncTestAnnotationPaste {
    # One actual four-cell WorksheetChange, with no locked helper columns.
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session,
          [Parameter(Mandatory)][int]$Row, [Parameter(Mandatory)][string]$Details,
          [Parameter(Mandatory)][string]$Person, [Parameter(Mandatory)][datetime]$Period,
          [Parameter(Mandatory)][string]$Article)
    [void](Assert-ExcelSyncTestCopy $Workbook $Session.test_root)
    if ($Row -lt 5) { throw 'Only data/input rows can receive the annotation paste.' }
    $sheet = $Workbook.Worksheets.Item([string]$Workbook.Worksheets.Item('_sync').Range('B3').Value2)
    $values = [object[,]]::new(1,4)
    $values[0,0]=$Details; $values[0,1]=$Person; $values[0,2]=$Period.ToOADate(); $values[0,3]=$Article
    $putArguments = [object[]]::new(1)
    $putArguments[0] = $values
    $range = $sheet.Range("AK${Row}:AN${Row}")
    $app = $Workbook.Application
    $oldEvents = $app.EnableEvents
    try {
        $app.EnableEvents = $true
        [void]$range.GetType().InvokeMember('Value2', [Reflection.BindingFlags]::SetProperty,
            $null, $range, $putArguments, [Globalization.CultureInfo]::GetCultureInfo('en-US'))
    } finally { $app.EnableEvents = $oldEvents }
    return Read-ExcelSyncTestState $Session
}

function Invoke-ExcelSyncTestSync {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
    [void](Assert-ExcelSyncTestCopy $Workbook $Session.test_root)
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.SyncNow')
    return [pscustomobject]@{
        status=(Get-ExcelSyncTestStatus $Workbook);
        state=(Read-ExcelSyncTestState $Session)
    }
}

function Add-ExcelSyncConfirmedTestCreate {
    # Starts from the exact durable state AFTER explicit creation confirmation.
    # The actual Add/Create confirmation UI is a separate test owned by the caller.
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session,
          [Parameter(Mandatory)][hashtable]$Fields)
    [void](Assert-ExcelSyncTestCopy $Workbook $Session.test_root)
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync')
    $state = Read-ExcelSyncTestState $Session
    $uuid = [guid]::NewGuid().ToString(); $key = 'u:' + $uuid
    $canonical = [ordered]@{
        transaction_date='2026-09-09T12:34:56';amount='-123.45';transaction_type='DEBIT';currency='UZS';
        operator_raw='native-fixture';application_mapped=$null;balance_after=$null;card_last_4='9876';
        receiver_name=$null;receiver_card=$null;is_p2p=$false;raw_message='isolated native Excel fixture';
        annotation_details=$null;annotation_person_code=$null;annotation_period=$null;annotation_article_code=$null
    }
    foreach ($field in $Fields.Keys) {
        if (-not $canonical.Contains($field)) { throw "Unsupported create field: $field" }
        $canonical[$field]=$Fields[$field]
    }
    $record = [ordered]@{id='';uuid=$uuid;version='';fields=$canonical;metadata=@{}}
    $state.records | Add-Member -MemberType NoteProperty -Name $key -Value $record
    $state.intents | Add-Member -MemberType NoteProperty -Name $key -Value @{kind='create';fields=$canonical}
    Write-ExcelSyncTestState $Session $state
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StartSync')
    return $key
}

function Add-ExcelSyncConfirmedTestDelete {
    # Likewise tests the deletion engine from already-confirmed durable intent.
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session, [Parameter(Mandatory)][string]$Id)
    [void](Assert-ExcelSyncTestCopy $Workbook $Session.test_root)
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync')
    $state = Read-ExcelSyncTestState $Session
    $entry = @($state.records.PSObject.Properties | Where-Object {[string]$_.Value.id -ceq $Id})
    if ($entry.Count -ne 1) { throw 'Expected exactly one baseline record.' }
    $key = [string]$entry[0].Name
    if ($state.intents.PSObject.Properties[$key] -or $state.blocked.PSObject.Properties[$key]) { throw 'Resolve existing changes first.' }
    $state.intents | Add-Member -MemberType NoteProperty -Name $key -Value @{kind='delete';fields=@{}}
    Write-ExcelSyncTestState $Session $state
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StartSync')
}

function Remove-ExcelSyncTestClient {
    param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
    [void](Assert-ExcelSyncTestCopy $Workbook $Session.test_root)
    if ($Session.test_marker -ne 'TBSparcer isolated native Excel test') { throw 'Missing test ownership marker.' }
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync')
    [void](Invoke-ExcelSyncMacro $Workbook 'SyncWindows.DeleteCredential' @([string]$Session.workbook_id))
    # A missing VBA String can marshal as a null BSTR in .NET. Verify the exact
    # synthetic target through cmdkey instead; /list never returns secret blobs.
    $credentialTarget = 'TBSparcer.ExcelSync.' + [string]$Session.workbook_id
    $cmdkey = Join-Path $env:SystemRoot 'System32\cmdkey.exe'
    $listing = (& $cmdkey ('/list:' + $credentialTarget) 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw 'Could not verify test credential removal through cmdkey.' }
    # cmdkey echoes the requested target in its first heading even when absent.
    $details = (($listing -split '\r?\n' | Where-Object {$_.Trim().Length -gt 0} | Select-Object -Skip 1) -join "`n")
    if ($details.IndexOf($credentialTarget, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw 'The test credential was not removed.'
    }
    # Only exact files in this freshly generated test UUID directory are removed.
    $directory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath([string]$Session.state_path))
    $expected = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ('TBSparcer\ExcelSync\' + [string]$Session.workbook_id)
    if ($directory -ne [IO.Path]::GetFullPath($expected)) { throw 'Unexpected test cleanup path.' }
    foreach ($name in @('state.json','state.json.tmp','client.lock')) {
        $target = Join-Path $directory $name
        if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target }
    }
    Remove-Item -LiteralPath $directory
    # Excel/workbook lifetime belongs to the caller. No Close/Quit or process kill.
}

Export-ModuleMember -Function Invoke-ExcelSyncMacro, Initialize-ExcelSyncTestClient, Read-ExcelSyncTestState,
    Invoke-ExcelSyncFixtureControl, Get-ExcelSyncTestRow, Set-ExcelSyncTestFields, Invoke-ExcelSyncTestSync,
    Add-ExcelSyncConfirmedTestCreate, Add-ExcelSyncConfirmedTestDelete, Remove-ExcelSyncTestClient,
    Test-ExcelSyncTestCredentialExists, Initialize-ExcelSyncSharedTestProfile, Get-ExcelSyncTestStatus,
    Set-ExcelSyncTestRowFields, Set-ExcelSyncTestAnnotationPaste
