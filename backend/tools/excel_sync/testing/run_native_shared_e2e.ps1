param(
    [Parameter(Mandatory)]$Workbook,
    [Parameter(Mandatory)][string]$FixturePath,
    [Parameter(Mandatory)][string]$TestRoot,
    [switch]$IncludeSaveAsTimer
)
# The caller owns Excel and opens a fresh test copy inside TestRoot.
# At phase awaiting_native_ui_close, the caller closes ONLY this copy through
# native Excel UI. The runner verifies its saved file and reopens the same copy.
# Excel's COM Close call has different nested-save behavior on Excel 16, so it
# cannot substitute for the user-close event in this test.
# It never starts/quits Excel or changes trust/automation-security settings.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force

$sharedBook = $Workbook
$sharedApp = $Workbook.Application
$sharedOriginalEvents = $sharedApp.EnableEvents
$sharedSession = $null
$sharedClosed = $false
$sharedFixtureValidated = $false
$sharedReportPath = Join-Path $TestRoot 'native-shared-e2e-verification.json'
$sharedReport = [ordered]@{
    native=$true; passed=$false; phase='prepare'; checks=@();
    workbook_path=[string]$Workbook.FullName; row_actions_ui_tested=$false
}

function Shared-Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Shared-Entries($Map) {
    if ($null -eq $Map) { return @() }
    return @($Map.PSObject.Properties | ForEach-Object { $_ })
}
function Shared-Phase([string]$Phase) {
    $script:sharedReport.phase=$Phase
    $script:sharedReport.updated_utc=[datetime]::UtcNow.ToString('o')
    $script:sharedReport | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $script:sharedReportPath -Encoding UTF8
}
function Shared-Client { return Read-ExcelSyncTestState $script:sharedSession }
function Shared-Server { return Invoke-ExcelSyncFixtureControl $script:sharedSession GET '/__test__/state' }
function Shared-Fault([hashtable]$Values) { [void](Invoke-ExcelSyncFixtureControl $script:sharedSession POST '/__test__/fault' $Values) }
function Shared-Sync { return Invoke-ExcelSyncTestSync $script:sharedBook $script:sharedSession }
function Shared-Stop { [void](Invoke-ExcelSyncMacro $script:sharedBook 'ExcelSync.StopSync') }
function Shared-Grid {
    return $script:sharedBook.Worksheets.Item([string]$script:sharedBook.Worksheets.Item('_sync').Range('B3').Value2)
}
function Shared-Record($Server, [string]$Id) {
    $found = @($Server.records | Where-Object {[string]$_.id -ceq $Id})
    Shared-Assert ($found.Count -eq 1) 'Expected exactly one fixture receipt by ID.'
    return $found[0]
}
function Shared-Queue($State) {
    return ([ordered]@{intents=$State.intents;outbox=$State.outbox;blocked=$State.blocked} | ConvertTo-Json -Depth 100 -Compress)
}
function Shared-EmptyQueue {
    $state = Shared-Client
    foreach ($field in @('intents','outbox','blocked')) {
        Shared-Assert (@(Shared-Entries $state.$field).Count -eq 0) ('Expected an empty ' + $field + ' map.')
    }
}
function Shared-CheckSaved {
    Shared-Assert ([bool]$script:sharedBook.Saved) 'Automatic SaveView did not leave the workbook saved.'
    $config = $script:sharedBook.Worksheets.Item('_sync')
    $status = Get-ExcelSyncTestStatus $script:sharedBook
    Shared-Assert ($status.Length -gt 0 -and $status -ceq [string]$config.Range('B9').Value2) 'Main status and hidden status mirror differ.'
    Shared-Assert ([string]$script:sharedBook.ActiveSheet.Name -ceq [string]$config.Range('B3').Value2) 'The data sheet is not active.'
}
function Shared-InputRow {
    $sheet = Shared-Grid
    $row = [Math]::Max(5, $sheet.Cells.Item($sheet.Rows.Count,52).End(-4162).Row + 1)
    Shared-Assert ([string]::IsNullOrEmpty([string]$sheet.Cells.Item($row,51).Value2) -and [string]::IsNullOrEmpty([string]$sheet.Cells.Item($row,52).Value2)) 'Bottom input row already has an identity.'
    foreach ($column in @(17,20,21,23,26,33,34,35,37,38,39,40,41,42,43,44)) {
        Shared-Assert (-not [bool]$sheet.Cells.Item($row,$column).Locked) 'The bottom input row is not editable.'
    }
    Shared-Assert ([bool]$sheet.Cells.Item($row+1,17).Locked) 'More than one blank input row is unlocked.'
    Shared-Assert ([double]$sheet.Rows.Item($row).RowHeight -eq 12) 'The blank input row lost its 12-point height.'
    return [int]$row
}
function Shared-DraftKey([int]$Row) {
    $sheet = Shared-Grid
    $uuid = [string]$sheet.Cells.Item($Row,52).Value2
    $parsed = [guid]::Empty
    Shared-Assert ([guid]::TryParse($uuid,[ref]$parsed)) 'Editing the blank row did not assign a draft UUID.'
    Shared-Assert ([string]::IsNullOrEmpty([string]$sheet.Cells.Item($Row,51).Value2)) 'An unsent draft already has a server ID.'
    $key = 'u:' + $uuid
    Shared-Assert ($null -ne (Shared-Client).records.PSObject.Properties[$key]) 'Draft was not persisted in local records.'
    return $key
}
function Shared-FinishStaging([hashtable]$ExtraFaults = @{}) {
    # Cancel debounce before lifting the fixture outage, so a background timer
    # cannot consume a one-shot failure before the deterministic test tick.
    Shared-Stop
    $faults = @{offline=$false}
    foreach ($key in $ExtraFaults.Keys) { $faults[$key]=$ExtraFaults[$key] }
    Shared-Fault $faults
    [void](Shared-Sync)
}
function Shared-CanonicalText($Value, [string]$Field) {
    if (($Value -is [datetime] -or $Value -is [datetimeoffset]) -and $Field -in @('transaction_date','annotation_period')) {
        $format = if ($Field -eq 'annotation_period') {'yyyy-MM-dd'} else {"yyyy-MM-dd'T'HH:mm:ss.FFFFFFF"}
        return $Value.ToString($format,[Globalization.CultureInfo]::InvariantCulture)
    }
    return [string]$Value
}
function Shared-StoredCellText([string]$Path, [string]$SheetName, [string]$Address) {
    # Read the closed test XLSM, not Excel's recovered in-memory sidecar view.
    $resolved=[IO.Path]::GetFullPath($Path)
    $root=[IO.Path]::GetFullPath($script:sharedSession.test_root).TrimEnd('\','/')
    Shared-Assert ($resolved.StartsWith($root + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) 'Unexpected stored workbook path.'
    Shared-Assert ($Address -match '^[A-Z]{1,3}[1-9][0-9]*$') 'Unexpected stored cell address.'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip=[IO.Compression.ZipFile]::OpenRead($resolved)
    function Read-SharedZipXml([string]$Name) {
        $entry=$zip.GetEntry($Name)
        if ($null -eq $entry) { throw 'Expected OOXML part is absent.' }
        $reader=[IO.StreamReader]::new($entry.Open(),[Text.Encoding]::UTF8)
        try { return [xml]$reader.ReadToEnd() } finally { $reader.Dispose() }
    }
    try {
        $bookXml=Read-SharedZipXml 'xl/workbook.xml'
        $sheet=@($bookXml.SelectNodes('/*[local-name()="workbook"]/*[local-name()="sheets"]/*[local-name()="sheet"]') | Where-Object {$_.GetAttribute('name') -ceq $SheetName})
        Shared-Assert ($sheet.Count -eq 1) 'Expected saved data sheet is absent.'
        $rid=$sheet[0].GetAttribute('id','http://schemas.openxmlformats.org/officeDocument/2006/relationships')
        $rels=Read-SharedZipXml 'xl/_rels/workbook.xml.rels'
        $rel=@($rels.SelectNodes('/*[local-name()="Relationships"]/*[local-name()="Relationship"]') | Where-Object {$_.GetAttribute('Id') -ceq $rid})
        Shared-Assert ($rel.Count -eq 1) 'Expected saved data relationship is absent.'
        $target=$rel[0].GetAttribute('Target')
        $part=if ($target.StartsWith('/')) {$target.TrimStart('/')} else {'xl/' + $target}
        $sheetXml=Read-SharedZipXml $part
        $cell=$sheetXml.SelectSingleNode('/*[local-name()="worksheet"]/*[local-name()="sheetData"]/*[local-name()="row"]/*[local-name()="c"][@r="' + $Address + '"]')
        if ($null -eq $cell) { return '' }
        Shared-Assert ($null -eq $cell.SelectSingleNode('./*[local-name()="f"]')) 'Stored literal text became a formula.'
        if ($cell.GetAttribute('t') -ceq 's') {
            $strings=Read-SharedZipXml 'xl/sharedStrings.xml'
            $index=[int]$cell.SelectSingleNode('./*[local-name()="v"]').InnerText
            $item=$strings.SelectNodes('/*[local-name()="sst"]/*[local-name()="si"]')[$index]
            return (($item.SelectNodes('.//*[local-name()="t"]') | ForEach-Object {$_.InnerText}) -join '')
        }
        if ($cell.GetAttribute('t') -ceq 'inlineStr') {
            return (($cell.SelectNodes('./*[local-name()="is"]//*[local-name()="t"]') | ForEach-Object {$_.InnerText}) -join '')
        }
        return [string]$cell.SelectSingleNode('./*[local-name()="v"]').InnerText
    } finally { $zip.Dispose() }
}

try {
    Shared-Phase 'prepare_fresh_shared_profile'
    $sharedSession = Initialize-ExcelSyncSharedTestProfile $sharedBook $FixturePath $TestRoot
    $sharedFixtureValidated = $true
    $sharedReport.workbook_id=$sharedSession.workbook_id
    $sharedReport.session_path=$sharedSession.session_path
    $baseline = Shared-Server
    Shared-Assert ($baseline.synthetic -and [string]$baseline.instance_id -ceq $sharedSession.instance_id) 'Unexpected synthetic API instance.'
    Shared-Assert (-not $baseline.faults.offline) 'Start with an online fixture.'
    Shared-Assert (@($baseline.records).Count -eq 2) 'Start with the untouched two-receipt fixture.'
    $baselineFields = @{}
    foreach ($record in $baseline.records) { $baselineFields[[string]$record.id]=$record.fields | ConvertTo-Json -Depth 40 -Compress }
    $baselinePosts = @($baseline.requests | Where-Object {$_.method -eq 'POST' -and $_.path -eq '/api/excel/v1/sync'}).Count
    Shared-Assert (-not (Test-Path -LiteralPath $sharedSession.state_path)) 'The bootstrap state was preseeded.'
    Shared-Assert (Test-ExcelSyncTestCredentialExists $sharedSession.workbook_id) 'The activated computer has no credential.'
    Shared-Assert ([string]$sharedBook.Worksheets.Item('_sync').Range('B6').Value2 -ceq '') 'The key must not travel inside the file.'

    # This preparatory close has no live client: setup already called StopSync.
    # Reopen events stay enabled, so Workbook_Open must import the profile.
    Shared-Phase 'workbook_open_bootstrap'
    $sharedApp.EnableEvents=$false
    $sharedBook.Close($false)
    $sharedBook=$null; $sharedClosed=$true
    $sharedApp.EnableEvents=$true
    $sharedBook=$sharedApp.Workbooks.Open([string]$sharedSession.workbook_path,0,$false)
    $sharedClosed=$false
    Shared-Assert (Test-Path -LiteralPath $sharedSession.state_path) 'Workbook_Open did not create the local profile.'
    Shared-Assert ([bool](Invoke-ExcelSyncMacro $sharedBook 'ExcelSync.IsActivated')) 'Workbook_Open did not recognise the activated computer.'
    $boot = Shared-Client
    Shared-Assert ([string]$boot.owner_id -ceq '1' -and [string]$boot.server_url -ceq $sharedSession.api_url) 'Workbook_Open did not bind the configured owner and server.'
    # SyncNow supplies a deterministic clock tick; it is not a user button.
    [void](Shared-Sync)
    $boot = Shared-Client
    Shared-Assert (@(Shared-Entries $boot.records).Count -eq 2 -and -not $boot.paused) 'The first full snapshot was not applied.'
    $bootServer=Shared-Server
    Shared-Assert (@($bootServer.requests | Where-Object {$_.method -eq 'POST' -and $_.path -eq '/api/excel/v1/sync'}).Count -eq $baselinePosts) 'Read-only bootstrap sent a mutation.'
    foreach ($record in $bootServer.records) {
        $row = Get-ExcelSyncTestRow $sharedBook ([string]$record.id)
        Shared-Assert ([string](Shared-Grid).Cells.Item($row,52).Value2 -ceq [string]$record.uuid) 'Snapshot UUID did not reach the grid.'
        Shared-Assert ([string](Shared-Grid).Cells.Item($row,53).Value2 -ceq [string]$record.version) 'Snapshot version did not reach the grid.'
    }
    [void](Shared-InputRow)
    Shared-EmptyQueue; Shared-CheckSaved
    $sharedReport.checks += 'Fresh UUID on an activated computer, absent state, no key in the file -> real Workbook_Open binds the loopback profile -> owner-checked full GET without POST'

    Shared-Phase 'bidirectional_edit'
    $marker = [guid]::NewGuid().ToString('N')
    $unicode = '{"merchant":"\u041e\u0431\u0449\u0438\u0439 \u0447\u0435\u043a","text":"\u0422\u0435\u043a\u0441\u0442 \u0401\u0436\u0438\u043a \ud83d\udcb3"}' | ConvertFrom-Json
    $clientText = '=' + $unicode.text + ' shared-client-' + $marker
    $serverText = $unicode.text + ' shared-server-' + $marker
    $originalDetails = (Shared-Record $baseline '2').fields.annotation_details
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestFields $sharedBook $sharedSession '2' @{annotation_details=$clientText})
    Shared-FinishStaging
    Shared-Assert ((Shared-Record (Shared-Server) '2').fields.annotation_details -ceq $clientText) 'WorksheetChange did not update the API exactly.'
    $row=Get-ExcelSyncTestRow $sharedBook '2'
    Shared-Assert (-not [bool](Shared-Grid).Cells.Item($row,37).HasFormula) 'Literal equals text became a formula.'
    [void](Invoke-ExcelSyncFixtureControl $sharedSession POST '/__test__/server-edit' @{id=2;fields=@{annotation_details=$serverText}})
    [void](Shared-Sync)
    $row=Get-ExcelSyncTestRow $sharedBook '2'
    Shared-Assert ([string](Shared-Grid).Cells.Item($row,37).Value2 -ceq $serverText) 'Server edit did not refresh into Excel.'
    Shared-EmptyQueue; Shared-CheckSaved
    [void](Invoke-ExcelSyncFixtureControl $sharedSession POST '/__test__/server-edit' @{id=2;fields=@{annotation_details=$originalDetails}})
    [void](Shared-Sync)
    $sharedReport.checks += 'Native cell edit -> server; independent server edit -> native grid; exact Unicode/literal equals; automatic save'

    Shared-Phase 'single_cell_draft_and_automatic_create'
    $beforeCount=(Shared-Server).counts.transactions
    $singleRow=Shared-InputRow
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestRowFields $sharedBook $sharedSession $singleRow ([ordered]@{operator_raw=($unicode.merchant + ' single-' + $marker)}))
    $singleKey=Shared-DraftKey $singleRow
    Shared-FinishStaging
    Shared-Assert ((Shared-Server).counts.transactions -eq $beforeCount) 'Incomplete single-cell draft was sent to the server.'
    Shared-Assert (@(Shared-Entries (Shared-Client).outbox).Count -eq 0) 'Incomplete draft received a wire operation.'
    Shared-Assert ((Shared-Client).intents.PSObject.Properties[$singleKey].Value.kind -ceq 'draft') 'Incomplete input is not a durable draft.'
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestRowFields $sharedBook $sharedSession $singleRow ([ordered]@{
        transaction_date='2026-09-10T12:34:56';transaction_type='DEBIT';currency='UZS';
        amount='123.45';card_last_4='0007';annotation_details=('auto-single-' + $marker);
        raw_message=($unicode.text + "`n" + 'second line')
    }))
    $beforeLedger=(Shared-Server).counts.operations
    Shared-FinishStaging @{lose_next_sync_ack=$true}
    $pending=@(Shared-Entries (Shared-Client).outbox)
    Shared-Assert ($pending.Count -eq 1 -and $pending[0].Value.kind -ceq 'create') 'Automatic creation did not retain its lost-ACK operation.'
    $createOperationId=[string]$pending[0].Name
    $createPayload=$pending[0].Value | ConvertTo-Json -Depth 60 -Compress
    Shared-Assert ((Shared-Server).counts.transactions -eq ($beforeCount + 1)) 'Automatic create was not committed exactly once.'
    Shared-Assert (@(Shared-Entries (Shared-Client).records).Count -eq ($beforeCount + 1)) 'Lost create ACK exposed a duplicate local row.'
    Shared-Stop
    Shared-Fault @{fail_next_sync=1}
    [void](Shared-Sync)
    $pending=@(Shared-Entries (Shared-Client).outbox)
    Shared-Assert ($pending.Count -eq 1 -and [string]$pending[0].Name -ceq $createOperationId) 'Failed create retry changed its operation ID.'
    Shared-Assert (($pending[0].Value | ConvertTo-Json -Depth 60 -Compress) -ceq $createPayload) 'Failed create retry changed its immutable payload.'
    Shared-Assert (@(Shared-Entries (Shared-Client).records).Count -eq ($beforeCount + 1)) 'Retry snapshot revealed an editable server duplicate.'
    [void](Shared-Sync)
    $server=Shared-Server
    $created=@($server.records | Where-Object {$_.fields.annotation_details -ceq ('auto-single-' + $marker)})
    Shared-Assert ($created.Count -eq 1 -and $server.counts.operations -eq ($beforeLedger + 1)) 'Automatic create retry duplicated a receipt or ledger operation.'
    $singleId=[string]$created[0].id
    Shared-Assert ([string]$created[0].fields.amount -ceq '-123.45' -and [string]$created[0].fields.card_last_4 -ceq '0007') 'Automatic create lost amount sign/cents or leading-zero card.'
    Shared-Assert ((Shared-CanonicalText $created[0].fields.transaction_date 'transaction_date') -ceq '2026-09-10T12:34:56') 'Automatic create changed local transaction time.'
    Shared-EmptyQueue; Shared-CheckSaved
    $sharedReport.created_receipt_id=$singleId
    $sharedReport.checks += 'Single-cell bottom input -> durable incomplete draft -> six required fields -> automatic create; lost ACK/retry keeps one immutable operation and one receipt'

    Shared-Phase 'paste_draft_and_automatic_create'
    $refs=$sharedBook.Worksheets.Item('_refs')
    $person=[string]$refs.Range('A2').Value2; $article=[string]$refs.Range('C2').Value2
    Shared-Assert ($person.Length -gt 0 -and $article.Length -gt 0) 'Fixture references were not loaded.'
    $pasteRow=Shared-InputRow
    $pasteDetails='=auto-paste-' + $marker
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestAnnotationPaste $sharedBook $sharedSession $pasteRow $pasteDetails $person ([datetime]'2026-08-01') $article)
    $pasteKey=Shared-DraftKey $pasteRow
    Shared-FinishStaging
    Shared-Assert ((Shared-Server).counts.transactions -eq ($beforeCount + 1)) 'Incomplete pasted draft was sent early.'
    Shared-Assert ((Shared-Client).intents.PSObject.Properties[$pasteKey].Value.kind -ceq 'draft') 'Pasted annotations did not become one durable draft.'
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestRowFields $sharedBook $sharedSession $pasteRow ([ordered]@{
        transaction_date='2026-09-10T13:14:15';transaction_type='CREDIT';currency='USD';
        operator_raw=($unicode.merchant + ' paste-' + $marker);amount='77.01';card_last_4='0019'
    }))
    Shared-FinishStaging
    $server=Shared-Server
    $pasted=@($server.records | Where-Object {$_.fields.annotation_details -ceq $pasteDetails})
    Shared-Assert ($pasted.Count -eq 1 -and $server.counts.transactions -eq ($beforeCount + 2)) 'The pasted draft did not automatically create exactly one receipt.'
    Shared-Assert ([string]$pasted[0].fields.annotation_person_code -ceq $person -and [string]$pasted[0].fields.annotation_article_code -ceq $article) 'Paste detached the annotation reference codes.'
    Shared-Assert ((Shared-CanonicalText $pasted[0].fields.annotation_period 'annotation_period') -ceq '2026-08-01') 'Paste changed the annotation period.'
    $pasteId=[string]$pasted[0].id
    $sharedReport.paste_receipt_id=$pasteId
    Shared-EmptyQueue; Shared-CheckSaved
    [void](Shared-InputRow)
    $sharedReport.checks += 'One native AK:AN matrix paste -> one incomplete draft -> automatic create after required fields, preserving reference codes and period'

    Shared-Phase 'offline_queue_close_and_reopen'
    $offlineText='shared-offline-' + $marker
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestFields $sharedBook $sharedSession $singleId @{annotation_details=$offlineText})
    Shared-FinishStaging @{fail_next_sync=1}
    $pending=@(Shared-Entries (Shared-Client).outbox)
    Shared-Assert ($pending.Count -eq 1 -and $pending[0].Value.kind -ceq 'update') 'Pre-commit failure did not preserve an immutable update.'
    $updateOperationId=[string]$pending[0].Name
    Shared-Fault @{offline=$true}
    # This newer edit happens after SaveView from the failed sync. A correct
    # reopen alone is insufficient: the sidecar could conceal a missed save.
    Shared-Phase 'offline_newer_edit_before_close'
    $offlineText='shared-offline-beforeclose-' + $marker
    [void](Set-ExcelSyncTestFields $sharedBook $sharedSession $singleId @{annotation_details=$offlineText})
    $closeRow=Get-ExcelSyncTestRow $sharedBook $singleId
    $closeSheetName=[string](Shared-Grid).Name
    Shared-Assert (-not [bool]$sharedBook.Saved) 'The BeforeClose probe did not leave a new unsaved edit.'
    $queueBeforeClose=Shared-Queue (Shared-Client)
    $sharedApp.EnableEvents=$true
    # Test the real user-close callback. COM Close(false) and Close() suppress
    # nested saving on the verified Excel 16 runtime; native UI close succeeds.
    # The caller handles this UI action; the harness never sends window input.
    $sharedReport.ui_close_workbook=[string]$sharedSession.workbook_path
    Shared-Phase 'awaiting_native_ui_close'
    Write-Host ('Native UI close requested for isolated test copy: ' + [string]$sharedSession.workbook_path)
    $closeDeadline=[datetime]::UtcNow.AddSeconds(180)
    do {
        Start-Sleep -Milliseconds 250
        $stillOpen=@($sharedApp.Workbooks | Where-Object {[string]$_.FullName -ceq [string]$sharedSession.workbook_path}).Count -gt 0
    } while ($stillOpen -and [datetime]::UtcNow -lt $closeDeadline)
    Shared-Assert (-not $stillOpen) 'The isolated workbook was not closed through native UI within 180 seconds.'
    $sharedReport.ui_close_observed=$true
    $sharedBook=$null; $sharedClosed=$true
    Shared-Phase 'offline_closed_xlsm_verification'
    $lockPath=Join-Path ([IO.Path]::GetDirectoryName($sharedSession.state_path)) 'client.lock'
    $lock=[IO.File]::Open($lockPath,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $lock.Dispose()
    Shared-Assert ((Shared-Queue (Shared-Client)) -ceq $queueBeforeClose) 'BeforeClose changed or lost the durable queue.'
    Shared-Assert ((Shared-StoredCellText $sharedSession.workbook_path $closeSheetName ('AK' + $closeRow)) -ceq $offlineText) 'BeforeClose did not save the latest visible edit into the XLSM.'
    Shared-Phase 'offline_native_workbook_open'
    $sharedApp.Visible=$true
    $sharedBook=$sharedApp.Workbooks.Open([string]$sharedSession.workbook_path,0,$false)
    $sharedClosed=$false
    # No StartSync here: Workbook_Open must restore the persisted client.
    Shared-Assert ([string]$sharedBook.Worksheets.Item('_sync').Range('B1').Value2 -ceq $sharedSession.workbook_id) 'Workbook_Open changed the workbook identity.'
    Shared-Assert ((Shared-Queue (Shared-Client)) -ceq $queueBeforeClose) 'Workbook_Open changed the immutable queued operation.'
    $row=Get-ExcelSyncTestRow $sharedBook $singleId
    Shared-Assert ([string](Shared-Grid).Cells.Item($row,37).Value2 -ceq $offlineText) 'Reopen did not restore the unsent edit.'
    Shared-Assert (-not [bool](Shared-Grid).Cells.Item($row,37).Locked) 'Reopen did not restore runtime editing.'
    Shared-FinishStaging
    # A second tick sends the newer intent after the immutable older update
    # receives its acknowledgement; the newer edit must not be discarded.
    [void](Shared-Sync)
    $server=Shared-Server
    Shared-Assert ((Shared-Record $server $singleId).fields.annotation_details -ceq $offlineText) 'Recovered queue did not reach the server.'
    Shared-Assert (@($server.operations | Where-Object {$_.operation_id -ceq $updateOperationId}).Count -eq 1) 'Recovered queue did not reuse exactly one original operation.'
    Shared-EmptyQueue; Shared-CheckSaved
    $sharedReport.checks += 'Pre-commit failure -> immutable update plus newer unsaved edit -> BeforeClose saves latest cell into disk OOXML -> offline Open restores same queue/grid -> original operation and newer intent both survive'

    if ($IncludeSaveAsTimer) {
    Shared-Phase 'automatic_timer_after_save_as'
    $oldWorkbookPath=[string]$sharedSession.workbook_path
    $renamedPath=Join-Path $sharedSession.test_root 'shared-smoke-saved-as.xlsm'
    Shared-Assert (-not (Test-Path -LiteralPath $renamedPath)) 'The SaveAs test destination already exists.'
    $sharedApp.EnableEvents=$true
    $sharedBook.SaveAs($renamedPath,52)
    $sharedSession.workbook_path=$renamedPath
    $sharedSession | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $sharedSession.session_path -Encoding UTF8
    $sharedReport.workbook_path=$renamedPath
    Shared-Assert ([string]$sharedBook.Worksheets.Item('_sync').Range('B1').Value2 -ceq $sharedSession.workbook_id) 'SaveAs changed the workbook identity.'
    Shared-CheckSaved
    $timerText=$unicode.text + ' shared-timer-' + $marker
    [void](Invoke-ExcelSyncFixtureControl $sharedSession POST '/__test__/server-edit' @{id=$pasteId;fields=@{receiver_name=$timerText}})
    $timerRow=Get-ExcelSyncTestRow $sharedBook $pasteId
    $timerStart=[datetime]::UtcNow
    $sharedReport.timer_started_utc=$timerStart.ToString('o')
    Shared-Phase 'waiting_for_real_60_second_ontime'
    $timerObserved=$false
    do {
        Start-Sleep -Milliseconds 500
        try {
            $timerObserved=([string](Shared-Grid).Cells.Item($timerRow,42).Value2 -ceq $timerText)
        } catch {
            # A short native timer callback may momentarily reject automation.
            if ($_.Exception.HResult -notin @(-2147418111,-2147417846)) { throw }
        }
    } while (-not $timerObserved -and ([datetime]::UtcNow-$timerStart).TotalSeconds -lt 85)
    $sharedReport.timer_wait_seconds=[Math]::Round(([datetime]::UtcNow-$timerStart).TotalSeconds,2)
    Shared-Assert $timerObserved 'Native OnTime did not refresh after SaveAs without a manual sync.'
    Shared-Assert (@($sharedApp.Workbooks | Where-Object {[string]$_.FullName -ceq $oldWorkbookPath}).Count -eq 0) 'The stale timer reopened the previous file name.'
    Shared-EmptyQueue; Shared-CheckSaved
    $sharedReport.checks += 'Native SaveAs preserves UUID; real 60-second OnTime receives an independent server edit without SyncNow; the old file is not reopened'
    }

    Shared-Phase 'conflict_without_overwrite'
    $localConflict='shared-local-conflict-' + $marker
    $serverConflict='shared-server-conflict-' + $marker
    Shared-Fault @{offline=$true}
    [void](Set-ExcelSyncTestFields $sharedBook $sharedSession $singleId @{annotation_details=$localConflict})
    Shared-Stop
    [void](Invoke-ExcelSyncFixtureControl $sharedSession POST '/__test__/server-edit' @{id=$singleId;fields=@{annotation_details=$serverConflict}})
    Shared-Fault @{offline=$false}
    [void](Shared-Sync)
    $conflicted=Shared-Client
    $blocked=@(Shared-Entries $conflicted.blocked)
    Shared-Assert ($blocked.Count -eq 1 -and $blocked[0].Value.status -ceq 'conflict') 'Concurrent edits did not retain a conflict.'
    Shared-Assert ((Shared-Record (Shared-Server) $singleId).fields.annotation_details -ceq $serverConflict) 'Conflict overwrote the server value.'
    $row=Get-ExcelSyncTestRow $sharedBook $singleId
    Shared-Assert ([string](Shared-Grid).Cells.Item($row,37).Value2 -ceq $localConflict) 'Conflict discarded the local proposed value.'
    $conflictQueue=Shared-Queue $conflicted
    $beforeRetry=(Shared-Server).counts.operations
    [void](Shared-Sync)
    Shared-Assert ((Shared-Queue (Shared-Client)) -ceq $conflictQueue) 'Background conflict retry changed the retained operation.'
    Shared-Assert ((Shared-Server).counts.operations -eq $beforeRetry) 'Blocked conflict replayed another mutation.'
    Shared-CheckSaved
    [void](Shared-InputRow)
    $final=Shared-Server
    foreach ($id in $baselineFields.Keys) {
        Shared-Assert (((Shared-Record $final $id).fields | ConvertTo-Json -Depth 40 -Compress) -ceq $baselineFields[$id]) 'A seed receipt did not retain its original field values.'
    }
    $grid=Shared-Grid
    $last=$grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row+1
    Shared-Assert ([double]$grid.Range("O5:BA$last").RowHeight -eq 12) 'Data/input rows lost their 12-point height.'
    $compactTextColumns = @(19,21,23,37,38,40,42,43,44,45,46,47,48,50)
    for ($col=15; $col -le 53; $col++) {
        $range = $grid.Range($grid.Cells.Item(5,$col),$grid.Cells.Item($last,$col))
        $clipText = $compactTextColumns -contains $col
        Shared-Assert ($range.WrapText -eq $clipText) "Column $col lost its text clipping policy."
        if ($clipText) {
            Shared-Assert ($range.VerticalAlignment -eq -4160 -and $range.ShrinkToFit -eq $false) "Column $col must show its first line without shrinking text."
        }
    }
    $sharedReport.checks += 'Concurrent edit remains conflicted with the server value preserved; later sync leaves the same blocked queue; all data/input rows remain 12pt with long text clipped at its original font size'
    $sharedReport.final_records=$final.counts.transactions
    $sharedReport.final_pending=@(Shared-Entries (Shared-Client).outbox).Count
    $sharedReport.final_conflicts=@(Shared-Entries (Shared-Client).blocked).Count
    $sharedReport.pending_ui_test='Double-click AX on created_receipt_id to resolve the retained conflict; use paste_receipt_id for confirmed deletion. Only synthetic created receipts are affected.'
    $sharedReport.passed=$true
    Shared-Phase 'complete'
} catch {
    $sharedReport.error=$_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}', '[redacted]'
    $sharedReport.error_line=$_.InvocationInfo.ScriptLineNumber
    throw 'Shared native E2E failed; inspect native-shared-e2e-verification.json.'
} finally {
    if ($sharedFixtureValidated) {
        try { Shared-Fault @{offline=$false;fail_next_snapshot=0;fail_next_sync=0;lose_next_sync_ack=$false} } catch {}
    }
    if ($sharedClosed -and $null -ne $sharedSession) {
        try {
            $sharedApp.EnableEvents=$true
            $sharedBook=$sharedApp.Workbooks.Open([string]$sharedSession.workbook_path,0,$false)
            $sharedClosed=$false
        } catch { $sharedReport.reopen_recovery_failed=$true }
    }
    if (-not $sharedReport.passed -and $null -ne $sharedBook) {
        try { Shared-Stop } catch {}
    }
    $sharedApp.EnableEvents=$sharedOriginalEvents
    $sharedReport | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $sharedReportPath -Encoding UTF8
}
[pscustomobject]$sharedReport
