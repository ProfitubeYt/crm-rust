param(
    [Parameter(Mandatory)]$Workbook,
    [Parameter(Mandatory)]$Session
)
# Run against the existing synthetic viewport session. The caller owns Excel.
# Both drafts are made through actual cell edits; no local-state injection,
# VBProject access, Excel startup/shutdown, or production connection is used.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if ($Session -is [string]) { $Session = Get-Content -LiteralPath $Session -Raw -Encoding UTF8 | ConvertFrom-Json }
$doBook = $Workbook
$doApp = $Workbook.Application
$doOriginalEvents = $doApp.EnableEvents
$doOriginalScreen = $doApp.ScreenUpdating
$doFixtureValidated = $false
$doReportPath = Join-Path $Session.test_root 'native-draft-order-verification.json'
$doReport = [ordered]@{native=$true;passed=$false;phase='validate';workbook_id=$Session.workbook_id;checks=@()}

function D-Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function D-Phase([string]$Phase) {
    $script:doReport.phase = $Phase
    $script:doReport.updated_utc = [datetime]::UtcNow.ToString('o')
    $script:doReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $script:doReportPath -Encoding UTF8
}
function D-State { return Read-ExcelSyncTestState $script:Session }
function D-Server { return Invoke-ExcelSyncFixtureControl $script:Session GET '/__test__/state' }
function D-Fault([bool]$Offline) {
    [void](Invoke-ExcelSyncFixtureControl $script:Session POST '/__test__/fault' @{offline=$Offline})
}
function D-Sync { [void](Invoke-ExcelSyncTestSync $script:doBook $script:Session) }
function D-Grid { return $script:doBook.Worksheets.Item([string]$script:doBook.Worksheets.Item('_sync').Range('B3').Value2) }
function D-Last { $grid=D-Grid; return [int]$grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row }
function D-EmptyQueue {
    $state = D-State
    foreach ($field in @('intents','outbox','blocked')) {
        D-Assert (@($state.$field.PSObject.Properties).Count -eq 0) ('Expected empty ' + $field)
    }
}
function D-DraftAt([int]$Row) {
    $grid = D-Grid
    $uuid = [string]$grid.Cells.Item($Row,52).Value2
    $parsed = [guid]::Empty
    D-Assert ([guid]::TryParse($uuid,[ref]$parsed)) 'Native cell edit did not assign a draft UUID.'
    D-Assert ([string]::IsNullOrEmpty([string]$grid.Cells.Item($Row,51).Value2)) 'An unacknowledged draft already has a server ID.'
    $key = 'u:' + $uuid
    $entry = (D-State).intents.PSObject.Properties[$key]
    D-Assert ($null -ne $entry -and [string]$entry.Value.kind -ceq 'draft') 'Expected a durable draft produced by the real SheetChange handler.'
    return $uuid
}
function D-RowForUuid([string]$Uuid) {
    $grid = D-Grid
    $values = $grid.Range('AZ5:AZ' + (D-Last)).Value2
    for ($i=1; $i -le $values.GetLength(0); $i++) {
        if ([string]$values.GetValue($i,1) -ceq $Uuid) { return $i+4 }
    }
    throw 'The expected draft UUID is missing from the grid.'
}
function D-Order {
    $grid = D-Grid
    $values = $grid.Range('AZ5:AZ' + (D-Last)).Value2
    $order = @()
    for ($i=1; $i -le $values.GetLength(0); $i++) { $order += [string]$values.GetValue($i,1) }
    return ,$order
}
function D-View($Window, $Grid) {
    $active = $Window.ActiveCell
    $panes = @()
    for ($i=1; $i -le [int]$Window.Panes.Count; $i++) {
        $pane = $Window.Panes.Item($i)
        D-Assert ($null -ne $pane.ScrollRow -and $null -ne $pane.ScrollColumn) 'Could not inspect the native pane.'
        $panes += [ordered]@{scroll_row=[int]$pane.ScrollRow;scroll_column=[int]$pane.ScrollColumn}
    }
    return [pscustomobject][ordered]@{
        active=[string]$active.Address($false,$false);active_key=[string]$Grid.Cells.Item([int]$active.Row,52).Value2;
        selection=[string]$Window.RangeSelection.Address($false,$false);
        scroll_row=[int]$Window.ScrollRow;scroll_column=[int]$Window.ScrollColumn;
        top_key=[string]$Grid.Cells.Item([int]$Window.ScrollRow,52).Value2;
        zoom=[int]$Window.Zoom;freeze=[bool]$Window.FreezePanes;
        split_row=[double]$Window.SplitRow;split_column=[double]$Window.SplitColumn;panes=$panes
    }
}
function D-ExactView($Before, $After, [hashtable]$Aliases) {
    foreach ($field in @('active','selection','scroll_row','scroll_column','top_key','zoom','freeze','split_row','split_column')) {
        D-Assert ([string]$Before.$field -ceq [string]$After.$field) ('Create acknowledgements changed ' + $field)
    }
    $expectedKey = [string]$Before.active_key
    if ($Aliases.ContainsKey($expectedKey)) { $expectedKey = [string]$Aliases[$expectedKey] }
    D-Assert ($expectedKey -ceq [string]$After.active_key) 'The active draft did not retain its own receipt after acknowledgement.'
    D-Assert (($Before.panes | ConvertTo-Json -Compress) -ceq ($After.panes | ConvertTo-Json -Compress)) 'Create acknowledgements moved a pane.'
}

try {
    D-Phase 'validate_existing_synthetic_session'
    D-Assert ([string]$Session.test_marker -ceq 'TBSparcer isolated native Excel test') 'Expected a synthetic native session.'
    D-Assert ([string]$Session.api_url -ceq 'http://127.0.0.1:8767') 'Only the isolated loopback fixture is allowed.'
    $root = [IO.Path]::GetFullPath([string]$Session.test_root).TrimEnd('\','/')
    $path = [IO.Path]::GetFullPath([string]$doBook.FullName)
    D-Assert ($path.StartsWith($root + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) 'Workbook is outside the synthetic test root.'
    D-Assert ($path -ceq [IO.Path]::GetFullPath([string]$Session.workbook_path) -and -not [bool]$doBook.ReadOnly) 'Use the original writable synthetic workbook.'
    $config = $doBook.Worksheets.Item('_sync')
    D-Assert ([string]$config.Range('B1').Value2 -ceq [string]$Session.workbook_id) 'Workbook/session identity mismatch.'
    D-Assert ([string]$config.Range('B5').Value2 -ceq [string]$Session.api_url) 'The workbook contains a different server origin.'
    D-Assert ([int]$doApp.CutCopyMode -eq 0) 'Finish the current Excel copy/cut before running this test.'
    $server = D-Server
    D-Assert ($server.synthetic -and [string]$server.instance_id -ceq [string]$Session.instance_id) 'Synthetic server identity mismatch.'
    D-Assert (-not $server.faults.offline -and @($server.records).Count -ge 200) 'Use the online viewport fixture with at least 200 receipts.'
    $doFixtureValidated = $true
    D-EmptyQueue
    $doApp.EnableEvents = $true
    [void](Invoke-ExcelSyncMacro $doBook 'ExcelSync.StartSync')
    D-Sync
    D-EmptyQueue
    $grid = D-Grid
    $window = $doBook.Windows.Item(1)
    $doBook.Activate()
    $grid.Activate()
    $baseline = D-Server
    $beforeCount = [int]$baseline.counts.transactions
    $beforeOperations = [int]$baseline.counts.operations
    $beforeWorkbookOperations = @($baseline.operations | Where-Object {[string]$_.workbook_id -ceq [string]$Session.workbook_id}).Count
    $marker = [guid]::NewGuid().ToString('N')
    $nameA = 'draft-order-A-' + $marker
    $nameB = 'draft-order-B-' + $marker

    D-Phase 'two_native_incomplete_drafts'
    D-Fault $true
    $rowA = (D-Last) + 1
    [void](Set-ExcelSyncTestRowFields $doBook $Session $rowA ([ordered]@{operator_raw=$nameA}))
    $uuidA = D-DraftAt $rowA
    $rowB = (D-Last) + 1
    D-Assert ($rowB -eq $rowA+1) 'The second blank input row was not created immediately below the first draft.'
    [void](Set-ExcelSyncTestRowFields $doBook $Session $rowB ([ordered]@{operator_raw=$nameB}))
    $uuidB = D-DraftAt $rowB
    D-Assert ($uuidA -cne $uuidB) 'Two drafts received the same local UUID.'
    D-Assert ((D-Server).counts.transactions -eq $beforeCount) 'An incomplete draft was sent prematurely.'
    $draftState = D-State
    D-Assert (@($draftState.intents.PSObject.Properties).Count -eq 2) 'Expected exactly two real draft intents.'
    D-Assert (@($draftState.outbox.PSObject.Properties).Count -eq 0) 'An offline incomplete draft already has an operation.'

    D-Phase 'complete_drafts_and_choose_different_display_order'
    [void](Set-ExcelSyncTestRowFields $doBook $Session $rowA ([ordered]@{
        transaction_date='2099-01-01T12:34:56';transaction_type='DEBIT';currency='UZS';
        operator_raw=$nameA;card_last_4='0081';amount='81.25'
    }))
    [void](Set-ExcelSyncTestRowFields $doBook $Session $rowB ([ordered]@{
        transaction_date='2098-01-01T12:34:56';transaction_type='CREDIT';currency='USD';
        operator_raw=$nameB;card_last_4='0082';amount='82.50'
    }))
    D-Assert ((D-DraftAt $rowA) -ceq $uuidA -and (D-DraftAt $rowB) -ceq $uuidB) 'Completing fields replaced a local draft identity.'
    D-Assert ((D-Server).counts.transactions -eq $beforeCount) 'Fixture outage failed to prevent an early create.'
    # Without a deliberately different display order, replacing two keys in
    # insertion order can accidentally look correct even without UUID aliases.
    [void](Invoke-ExcelSyncMacro $doBook 'ExcelSync.SortByDate')
    $rowA = D-RowForUuid $uuidA
    $rowB = D-RowForUuid $uuidB
    D-Assert ($rowB+1 -eq $rowA -and $rowA -eq (D-Last)) 'The public sort did not arrange the two drafts as B then A.'
    $grid.Range('AH' + $rowB + ':AI' + $rowA).Select()
    $grid.Range('AI' + $rowA).Activate()
    $window.ScrollRow = 110
    $window.ScrollColumn = 35
    D-Assert ([int]$window.ActiveCell.Row -gt [int]$window.ScrollRow+30) 'The selected draft must be offscreen below the viewport.'
    $orderBefore = D-Order
    $viewBefore = D-View $window $grid
    D-Assert ($viewBefore.active_key -ceq $uuidA) 'The active cell is not the intended first-created draft.'
    $doReport.before = [ordered]@{view=$viewBefore;draft_a_row=$rowA;draft_b_row=$rowB;local_uuid_a=$uuidA;local_uuid_b=$uuidB}

    D-Phase 'acknowledge_both_creates_without_reordering'
    # No StopSync/StartSync here: the tested acknowledgement must run against
    # the live state and view produced by the real edits and the user's sort.
    D-Fault $false
    D-Sync
    $server = D-Server
    $createdA = @($server.records | Where-Object {[string]$_.fields.operator_raw -ceq $nameA})
    $createdB = @($server.records | Where-Object {[string]$_.fields.operator_raw -ceq $nameB})
    D-Assert ($createdA.Count -eq 1 -and $createdB.Count -eq 1) 'Each draft must create exactly one server receipt.'
    D-Assert ([string]$createdA[0].uuid -cne $uuidA -and [string]$createdB[0].uuid -cne $uuidB) 'The server did not replace the local draft UUIDs.'
    D-Assert ([string]$createdA[0].uuid -cne [string]$createdB[0].uuid) 'The server assigned duplicate receipt UUIDs.'
    D-Assert ([string]$createdA[0].fields.amount -ceq '-81.25' -and [string]$createdB[0].fields.amount -ceq '82.50') 'Draft amount/direction was changed incorrectly.'
    D-Assert ([int]$server.counts.transactions -eq $beforeCount+2) 'Acknowledgements created an unexpected number of receipts.'
    D-Assert ([int]$server.counts.operations -eq $beforeOperations+2) 'Expected exactly two newly committed operations.'
    $newOperations = @($server.operations | Where-Object {[string]$_.workbook_id -ceq [string]$Session.workbook_id} | Select-Object -Skip $beforeWorkbookOperations)
    D-Assert ($newOperations.Count -eq 2 -and @($newOperations | Where-Object {$_.status -cne 'applied'}).Count -eq 0) 'Both native create operations must be acknowledged as applied.'
    $aliases = @{}
    $aliases[$uuidA] = [string]$createdA[0].uuid
    $aliases[$uuidB] = [string]$createdB[0].uuid
    $expectedOrder = @($orderBefore | ForEach-Object {if ($aliases.ContainsKey([string]$_)) {$aliases[[string]$_]} else {[string]$_}})
    $orderAfter = D-Order
    D-Assert (($orderAfter -join ',') -ceq ($expectedOrder -join ',')) 'Create ACK reset the chosen row order instead of replacing identities in place.'
    D-Assert ((Get-ExcelSyncTestRow $doBook ([string]$createdA[0].id)) -eq $rowA) 'The first-created draft changed its chosen row.'
    D-Assert ((Get-ExcelSyncTestRow $doBook ([string]$createdB[0].id)) -eq $rowB) 'The second-created draft changed its chosen row.'
    $viewAfter = D-View $window $grid
    D-ExactView $viewBefore $viewAfter $aliases
    D-EmptyQueue
    D-Assert ([double]$grid.Range('O5:BA' + ((D-Last)+1)).RowHeight -eq 12) 'Draft acknowledgement changed the compact row height.'
    D-Assert ([bool]$doBook.Saved) 'Acknowledged drafts were not saved to the workbook.'
    $doReport.after = [ordered]@{view=$viewAfter;server_id_a=[string]$createdA[0].id;server_id_b=[string]$createdB[0].id;server_uuid_a=[string]$createdA[0].uuid;server_uuid_b=[string]$createdB[0].uuid}
    $doReport.checks += 'Two partial drafts created by SheetChange, completed offline, and sent through the real sync path'
    $doReport.checks += 'Two server-assigned UUIDs and exactly two applied operations, one receipt per draft'
    $doReport.checks += 'Chosen B-before-A sort, every existing UUID position, active draft, exact selection, viewport and panes preserved through both acknowledgements'
    $doReport.passed = $true
    D-Phase 'complete'
} catch {
    $doReport.error = $_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'
    $doReport.error_line = $_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    if ($doFixtureValidated) {
        try { [void](Invoke-ExcelSyncMacro $doBook 'ExcelSync.StopSync') }
        catch { $doReport.stop_error = $_.Exception.Message }
        try { D-Fault $false }
        catch { $doReport.fixture_cleanup_error = $_.Exception.Message }
    }
    $doApp.EnableEvents = $doOriginalEvents
    $doApp.ScreenUpdating = $doOriginalScreen
    $doReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $doReportPath -Encoding UTF8
}
[pscustomobject]$doReport
