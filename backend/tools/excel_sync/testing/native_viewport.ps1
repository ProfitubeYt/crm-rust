param(
    [Parameter(Mandatory)]$Workbook,
    [Parameter(Mandatory)][string]$FixturePath,
    [Parameter(Mandatory)][string]$TestRoot,
    [switch]$RecoveryTest
)
# Caller opens a fresh, empty test XLSM inside TestRoot. Use the isolated
# test_server.py --seed-count 240 on port 8767 and its unused credentials.json.
# Only this test copy is reopened. This runner never starts or quits Excel,
# changes macro trust, or connects to the application server.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
$vpBook = $Workbook
$vpApp = $Workbook.Application
$vpOriginalEvents = $vpApp.EnableEvents
$vpOriginalScreenUpdating = $vpApp.ScreenUpdating
$vpSession = $null
$vpOtherBook = $null
$vpOwnClipboard = $false
$vpExtraDeleted = 0
$vpReportPath = Join-Path $TestRoot 'native-viewport-verification.json'
$vpReport = [ordered]@{native=$true;passed=$false;phase='prepare';checks=@();skipped=@();snapshots=@();application_flags_recovery_tested=$false}

function V-Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function V-FileHash([string]$Path) {
    # Excel keeps the open XLSM writable. Get-FileHash's sharing mode can
    # reject that handle even though read access is available.
    $stream = $null
    $sha = $null
    try {
        $sharing = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
        $stream = [IO.FileStream]::new($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,$sharing)
        $sha = [Security.Cryptography.SHA256]::Create()
        return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','')
    } finally {
        if ($null -ne $sha) { $sha.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
    }
}
function V-Phase([string]$Phase) {
    $script:vpReport.phase = $Phase
    $script:vpReport.updated_utc = [datetime]::UtcNow.ToString('o')
    $script:vpReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $script:vpReportPath -Encoding UTF8
}
function V-Grid {
    return $script:vpBook.Worksheets.Item([string]$script:vpBook.Worksheets.Item('_sync').Range('B3').Value2)
}
function V-Server { return Invoke-ExcelSyncFixtureControl $script:vpSession GET '/__test__/state' }
function V-State { return Read-ExcelSyncTestState $script:vpSession }
function V-Sync { [void](Invoke-ExcelSyncTestSync $script:vpBook $script:vpSession) }
function V-EditServer([string]$Id, [hashtable]$Fields) {
    $result = Invoke-ExcelSyncFixtureControl $script:vpSession POST '/__test__/server-edit' @{id=[int]$Id;fields=$Fields}
    V-Assert ($result.status -in @('applied','noop')) 'Synthetic server edit was not applied.'
}
function V-ServerRecord($State, [string]$Id) {
    $records = @($State.records | Where-Object {[string]$_.id -ceq $Id})
    V-Assert ($records.Count -eq 1) 'Expected exactly one synthetic receipt.'
    return $records[0]
}
function V-KeyAt($Sheet, [int]$Row) {
    if ($Row -lt 5) { return 'header:' + $Row }
    return [string]$Sheet.Cells.Item($Row,52).Value2
}
function V-View($Window, $Sheet) {
    $selected = $Window.RangeSelection
    $active = $Window.ActiveCell
    $signature = @()
    for ($areaIndex=1; $areaIndex -le [int]$selected.Areas.Count; $areaIndex++) {
        $area = $selected.Areas.Item($areaIndex)
        $first = [int]$area.Row
        $last = $first + [int]$area.Rows.Count - 1
        $signature += ([ordered]@{
            first_key=(V-KeyAt $Sheet $first);last_key=(V-KeyAt $Sheet $last);
            column=[int]$area.Column;rows=[int]$area.Rows.Count;columns=[int]$area.Columns.Count
        } | ConvertTo-Json -Compress)
    }
    $top = [int]$Window.ScrollRow
    while ($top -lt 10000 -and [bool]$Sheet.Rows.Item($top).Hidden) { $top++ }
    $panes = @()
    # PowerShell 5's COM enumerator can return a Panes proxy rather than a
    # Pane. Indexed access is needed to inspect the actual scroll positions.
    for ($paneIndex=1; $paneIndex -le [int]$Window.Panes.Count; $paneIndex++) {
        $pane = $Window.Panes.Item($paneIndex)
        V-Assert ($null -ne $pane.ScrollRow -and $null -ne $pane.ScrollColumn) 'Could not read the native pane scroll positions.'
        $panes += [ordered]@{scroll_row=[int]$pane.ScrollRow;scroll_column=[int]$pane.ScrollColumn}
    }
    return [pscustomobject][ordered]@{
        sheet=[string]$Sheet.Name;active=[string]$active.Address($false,$false);
        active_key=(V-KeyAt $Sheet ([int]$active.Row));active_column=[int]$active.Column;
        selection=[string]$selected.Address($false,$false);selection_signature=($signature -join '|');
        scroll_row=[int]$Window.ScrollRow;scroll_column=[int]$Window.ScrollColumn;
        top_key=(V-KeyAt $Sheet $top);zoom=[int]$Window.Zoom;
        freeze=[bool]$Window.FreezePanes;split_row=[double]$Window.SplitRow;
        split_column=[double]$Window.SplitColumn;panes=$panes
    }
}
function V-ExactView($Before, $After, [string]$Context) {
    foreach ($field in @('sheet','active','active_key','active_column','selection','selection_signature',
                         'scroll_row','scroll_column','top_key','zoom','freeze','split_row','split_column')) {
        V-Assert ([string]$Before.$field -ceq [string]$After.$field) ($Context + ': changed ' + $field)
    }
    V-Assert (($Before.panes | ConvertTo-Json -Compress) -ceq ($After.panes | ConvertTo-Json -Compress)) ($Context + ': changed pane scroll positions')
}
function V-AnchorView($Before, $After, [string]$Context) {
    # Deletion can compact row numbers. Preserve the user's records and their
    # screen position, rather than requiring the old physical row addresses.
    foreach ($field in @('sheet','active_key','active_column','selection_signature','scroll_column',
                         'top_key','zoom','freeze','split_row','split_column')) {
        V-Assert ([string]$Before.$field -ceq [string]$After.$field) ($Context + ': changed ' + $field)
    }
}
function V-EmptyQueue {
    $state = V-State
    foreach ($field in @('intents','outbox','blocked')) {
        V-Assert (@($state.$field.PSObject.Properties).Count -eq 0) ('Expected empty ' + $field)
    }
}
function V-CheckGrid {
    $grid = V-Grid
    $last = [int]$grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row
    V-Assert ($last -ge 200) 'The regression needs at least 200 populated rows.'
    V-Assert ([double]$grid.Range('O5:BA' + ($last+1)).RowHeight -eq 12) 'Data or input row height changed from 12 points.'
    $compactTextColumns = @(19,21,23,37,38,40,42,43,44,45,46,47,48,50)
    for ($col=15; $col -le 53; $col++) {
        $range = $grid.Range($grid.Cells.Item(5,$col),$grid.Cells.Item($last+1,$col))
        $clipText = $compactTextColumns -contains $col
        V-Assert ($range.WrapText -eq $clipText) "Column $col lost its text clipping policy."
        if ($clipText) {
            V-Assert ($range.VerticalAlignment -eq -4160 -and $range.ShrinkToFit -eq $false) "Column $col must show its first line without shrinking text."
        }
    }
    V-Assert ([bool]$grid.Columns.Item('AY:BA').Hidden) 'Technical identity columns became visible.'
    V-Assert ([bool]$script:vpApp.EnableEvents) 'Synchronization disabled Excel events.'
    V-Assert ([bool]$script:vpApp.ScreenUpdating) 'Synchronization left screen updates disabled.'
}
function V-Order {
    $grid = V-Grid
    $last = [int]$grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row
    $values = $grid.Range('AZ5:AZ' + $last).Value2
    $ids = @()
    for ($i=1; $i -le $values.GetLength(0); $i++) { $ids += [string]$values.GetValue($i,1) }
    return $ids -join ','
}

try {
    V-Assert ([int]$vpApp.CutCopyMode -eq 0) 'Finish the current Excel copy/cut operation before running this isolated test.'
    V-Phase 'prepare_fresh_shared_test_copy'
    $vpSession = Initialize-ExcelSyncSharedTestProfile $vpBook $FixturePath $TestRoot
    $vpReport.workbook_id = $vpSession.workbook_id
    $vpReport.workbook_path = $vpSession.workbook_path
    $vpReport.session_path = $vpSession.session_path
    $server = V-Server
    V-Assert ($server.synthetic -and [string]$server.instance_id -ceq [string]$vpSession.instance_id) 'Synthetic fixture identity mismatch.'
    V-Assert (-not $server.faults.offline) 'Start with the fixture online.'
    V-Assert (@($server.records).Count -ge 220) 'Start a fresh fixture with --seed-count 240 (at least 220 records).'
    V-Assert ($server.counts.operations -eq 0) 'Use an untouched synthetic fixture.'
    $vpReport.seed_count = @($server.records).Count
    $belowId = [string]$server.records[-1].id

    # Setup has stopped the client, so this preparatory close has no unsaved
    # live queue. The actual reopen exercises Workbook_Open bootstrap.
    $vpApp.EnableEvents = $false
    $vpBook.Close($false)
    $vpBook = $null
    $vpApp.EnableEvents = $true
    $vpBook = $vpApp.Workbooks.Open([string]$vpSession.workbook_path,0,$false)
    V-Sync
    V-EmptyQueue
    $grid = V-Grid
    $window = $vpBook.Windows.Item(1)
    V-Assert (@((V-State).records.PSObject.Properties).Count -eq $vpReport.seed_count) 'Bootstrap did not retrieve every synthetic record.'
    V-CheckGrid

    V-Phase 'offscreen_selection_and_noop'
    $vpBook.Activate()
    $grid.Activate()
    $window.Zoom = 100
    $grid.Range('AH60:AI62').Select()
    $grid.Range('AI61').Activate()
    V-Assert ([string]$window.RangeSelection.Address($false,$false) -ceq 'AH60:AI62') 'Activating the inner cell unexpectedly changed the selected range.'
    V-Assert ([string]$window.ActiveCell.Address($false,$false) -ceq 'AI61') 'The active cell must be inside, not at the top left of, the selection.'
    $window.ScrollRow = 110
    $window.ScrollColumn = 35
    V-Assert ([int]$window.ScrollRow -gt [int]$window.ActiveCell.Row) 'The test selection must be above the visible viewport.'
    $vpBook.Save()
    V-Assert ([bool]$vpBook.Saved) 'Initial viewport setup was not saved.'
    $before = V-View $window $grid
    $fileHash = V-FileHash $vpSession.workbook_path
    $writeTime = (Get-Item -LiteralPath $vpSession.workbook_path).LastWriteTimeUtc.Ticks
    $operationCount = (V-Server).counts.operations
    for ($tick=0; $tick -lt 2; $tick++) {
        Start-Sleep -Milliseconds 1100
        V-Sync
        $after = V-View $window $grid
        $vpReport.snapshots += [ordered]@{case=('noop_' + $tick);before=$before;after=$after}
        V-ExactView $before $after 'No-op refresh'
        V-Assert ([bool]$vpBook.Saved) 'No-op refresh dirtied the workbook.'
        V-Assert ((V-FileHash $vpSession.workbook_path) -ceq $fileHash) 'No-op refresh rewrote the saved XLSM.'
        V-Assert ((Get-Item -LiteralPath $vpSession.workbook_path).LastWriteTimeUtc.Ticks -eq $writeTime) 'No-op refresh touched the file on disk.'
    }
    V-Assert ((V-Server).counts.operations -eq $operationCount) 'No-op refresh echoed a mutation.'
    $vpReport.checks += 'Two no-op ticks preserve offscreen multiselection, active record, every pane, zoom, Saved, XLSM SHA256 and write time'

    V-Phase 'receive_and_send_with_viewport'
    $marker = [guid]::NewGuid().ToString('N')
    $unicode = ('{"text":"\u0422\u0435\u0441\u0442 \u0401\u0436\u0438\u043a \ud83d\udcb3"}' | ConvertFrom-Json).text
    $incoming = $unicode + ' viewport-server-' + $marker
    V-EditServer '128' @{annotation_details=$incoming}
    $before = V-View $window $grid
    V-Sync
    V-ExactView $before (V-View $window $grid) 'Incoming edit'
    $incomingRow = Get-ExcelSyncTestRow $vpBook '128'
    V-Assert ([string]$grid.Cells.Item($incomingRow,37).Value2 -ceq $incoming) 'Server edit did not reach the displayed cell.'
    $outgoing = '=' + $unicode + ' viewport-client-' + $marker
    [void](Set-ExcelSyncTestFields $vpBook $vpSession '129' @{annotation_details=$outgoing})
    V-ExactView $before (V-View $window $grid) 'Local edit capture'
    V-Sync
    V-ExactView $before (V-View $window $grid) 'POST acknowledgement'
    V-Assert ([string](V-ServerRecord (V-Server) '129').fields.annotation_details -ceq $outgoing) 'Excel edit did not reach the server.'
    $outgoingRow = Get-ExcelSyncTestRow $vpBook '129'
    V-Assert (-not [bool]$grid.Cells.Item($outgoingRow,37).HasFormula) 'Literal annotation became a formula.'
    V-EmptyQueue
    $vpReport.checks += 'Unicode GET and actual SheetChange -> POST -> ACK preserve the viewport and literal text, with empty durable queues'

    V-Phase 'delete_above_and_below_anchor'
    $before = V-View $window $grid
    [void](Invoke-ExcelSyncFixtureControl $vpSession DELETE '/__test__/record/1')
    V-Sync
    $after = V-View $window $grid
    $vpReport.snapshots += [ordered]@{case='delete_above';before=$before;after=$after}
    V-AnchorView $before $after 'Deletion above viewport'
    V-Assert ([int]$after.scroll_row -eq ([int]$before.scroll_row-1)) 'Top record did not retain its screen position after compaction.'
    $before = $after
    [void](Invoke-ExcelSyncFixtureControl $vpSession DELETE ('/__test__/record/' + $belowId))
    V-Sync
    V-ExactView $before (V-View $window $grid) 'Deletion below viewport'
    $vpReport.checks += 'Deleting above the viewport preserves selected UUIDs and top-record anchor; deleting below leaves physical view unchanged'

    V-Phase 'new_bottom_input_row'
    $before = V-View $window $grid
    $input = [int]$grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row + 1
    $createName = 'viewport-create-' + $marker
    [void](Set-ExcelSyncTestRowFields $vpBook $vpSession $input ([ordered]@{
        operator_raw=$createName;card_last_4='0051';currency='UZS';
        transaction_date=[datetime]'2026-09-15T12:34:56';transaction_type='DEBIT';amount=25.5
    }))
    V-ExactView $before (V-View $window $grid) 'Bottom draft creation'
    V-Sync
    V-ExactView $before (V-View $window $grid) 'Create acknowledgement'
    $created = @((V-Server).records | Where-Object {$_.fields.operator_raw -ceq $createName})
    V-Assert ($created.Count -eq 1 -and [string]$created[0].fields.amount -ceq '-25.50') 'Bottom input did not create exactly one receipt.'
    V-EmptyQueue
    $vpReport.checks += 'Editing the actual bottom input row auto-creates exactly one receipt without moving the working viewport'

    V-Phase 'sort_order_and_optional_filter'
    V-EditServer '3' @{transaction_date='2030-01-01T12:30:00'}
    V-Sync
    [void](Invoke-ExcelSyncMacro $vpBook 'ExcelSync.SortByDate')
    $before = V-View $window $grid
    $order = V-Order
    V-EditServer '128' @{balance_after='7654.32'}
    V-Sync
    V-ExactView $before (V-View $window $grid) 'Refresh of sorted data'
    V-Assert ((V-Order) -ceq $order) 'Refresh reset the chosen row order.'
    if ([bool]$grid.AutoFilterMode) {
        $filterRange = $grid.AutoFilter.Range
        $field = 20 - [int]$filterRange.Column + 1
        V-Assert ($field -gt 0 -and $field -le [int]$filterRange.Columns.Count) 'Existing filter does not cover transaction type.'
        [void]$filterRange.AutoFilter($field,'DEBIT')
        V-Assert ([bool]$grid.FilterMode) 'The existing filter could not be applied.'
        $before = V-View $window $grid
        V-EditServer '129' @{balance_after='8765.43'}
        V-Sync
        V-ExactView $before (V-View $window $grid) 'Refresh of filtered data'
        V-Assert ([bool]$grid.FilterMode -and [bool]$grid.AutoFilter.Filters.Item($field).On) 'Refresh removed the filter.'
        V-Assert ([string]$grid.AutoFilter.Filters.Item($field).Criteria1 -match 'DEBIT') 'Refresh changed the filter criterion.'
        $grid.ShowAllData()
        $vpReport.checks += 'Existing AutoFilter criteria and viewport survive incoming changes'
    } else {
        $vpReport.skipped += 'AutoFilter: template has no AutoFilter; no filter UI was added by the test'
    }
    $vpReport.checks += 'Public SortByDate order remains stable on refresh'

    V-Phase 'other_workbook_keeps_focus'
    $targetBefore = V-View $window $grid
    $vpOtherBook = $vpApp.Workbooks.Add(-4167)
    $otherSheet = $vpOtherBook.Worksheets.Item(1)
    $otherSheet.Range('A1').Value2 = 'Disposable viewport regression workbook'
    $otherSheet.Range('D7:E8').Select()
    $otherWindow = $vpOtherBook.Windows.Item(1)
    $otherWindow.ScrollRow = 25
    $otherWindow.ScrollColumn = 9
    $otherBefore = V-View $otherWindow $otherSheet
    $otherName = [string]$vpOtherBook.Name
    $otherIncoming = 'inactive-receive-' + $marker
    V-EditServer '128' @{annotation_details=$otherIncoming}
    V-Sync
    V-Assert ([string]$vpApp.ActiveWorkbook.Name -ceq $otherName) 'Synchronization stole focus from another workbook.'
    V-ExactView $otherBefore (V-View $otherWindow $otherSheet) 'Other active workbook'
    V-ExactView $targetBefore (V-View $window $grid) 'Inactive target workbook'
    $vpBook.Activate()
    V-Sync
    $incomingRow = Get-ExcelSyncTestRow $vpBook '128'
    V-Assert ([string]$grid.Cells.Item($incomingRow,37).Value2 -ceq $otherIncoming) 'Background/deferred receive did not reach the target workbook.'
    $vpReport.checks += 'Background synchronization preserves both windows and never activates the target workbook'

    V-Phase 'clipboard_defers_and_timer_resumes'
    $before = V-View $window $grid
    $clipIncoming = 'clipboard-receive-' + $marker
    V-EditServer '128' @{annotation_details=$clipIncoming}
    $incomingRow = Get-ExcelSyncTestRow $vpBook '128'
    $oldText = [string]$grid.Cells.Item($incomingRow,37).Value2
    $window.RangeSelection.Copy()
    $vpOwnClipboard = $true
    V-Assert ([int]$vpApp.CutCopyMode -eq 1) 'The native range copy did not enter copy mode.'
    V-Sync
    V-Assert ([int]$vpApp.CutCopyMode -eq 1) 'Synchronization cancelled the active clipboard copy.'
    V-ExactView $before (V-View $window $grid) 'Deferred refresh during copy'
    V-Assert ([string]$grid.Cells.Item($incomingRow,37).Value2 -ceq $oldText) 'The copied grid changed before copy mode ended.'
    $vpApp.CutCopyMode = $false
    $vpOwnClipboard = $false
    $start = [datetime]::UtcNow
    $received = $false
    do {
        Start-Sleep -Milliseconds 300
        try { $received = ([string]$grid.Cells.Item($incomingRow,37).Value2 -ceq $clipIncoming) }
        catch {
            $code = $_.Exception.HResult
            if ($null -ne $_.Exception.InnerException) { $code = $_.Exception.InnerException.HResult }
            if ($code -notin @(-2147418111,-2147417846)) { throw }
        }
    } while (-not $received -and ([datetime]::UtcNow-$start).TotalSeconds -lt 20)
    $vpReport.clipboard_resume_seconds = [Math]::Round(([datetime]::UtcNow-$start).TotalSeconds,2)
    V-Assert $received 'Deferred synchronization did not resume through native OnTime within 20 seconds.'
    V-ExactView $before (V-View $window $grid) 'Deferred timer completion'
    $vpReport.checks += 'Native clipboard copy survives a sync tick; after cancellation real OnTime receives pending data without a manual SyncNow'

    if ($RecoveryTest) {
        V-Phase 'preserve_preexisting_application_flags'
        $priorEvents = [bool]$vpApp.EnableEvents
        $priorScreenUpdating = [bool]$vpApp.ScreenUpdating
        $recoveryIncoming = 'flags-recovery-' + $marker
        V-EditServer '128' @{annotation_details=$recoveryIncoming}
        $before = V-View $window $grid
        try {
            $vpApp.EnableEvents = $false
            $vpApp.ScreenUpdating = $false
            V-Sync
            V-Assert (-not [bool]$vpApp.EnableEvents) 'Synchronization overrode the caller''s disabled events flag.'
            V-Assert (-not [bool]$vpApp.ScreenUpdating) 'Synchronization overrode the caller''s disabled screen-updating flag.'
            $incomingRow = Get-ExcelSyncTestRow $vpBook '128'
            V-Assert ([string]$grid.Cells.Item($incomingRow,37).Value2 -ceq $recoveryIncoming) 'The flags recovery test did not execute a real incoming edit.'
            V-ExactView $before (V-View $window $grid) 'Synchronization with preexisting disabled application flags'
            $vpReport.application_flags_recovery_tested = $true
            $vpReport.checks += 'A real incoming edit preserves EnableEvents=False and ScreenUpdating=False; the runner restores both previous values in finally'
        } finally {
            $vpApp.EnableEvents = $priorEvents
            $vpApp.ScreenUpdating = $priorScreenUpdating
        }
    } else {
        $vpReport.skipped += 'Application flags recovery: not requested; pass -RecoveryTest to execute it'
    }

    V-Phase 'rejected_equal_value_formula_is_removed'
    V-EditServer '128' @{transaction_type='CREDIT';amount='30.00'}
    V-Sync
    $formulaRow = Get-ExcelSyncTestRow $vpBook '128'
    $amountCell = $grid.Cells.Item($formulaRow,26)
    V-Assert ([double]$amountCell.Value2 -eq 30) 'The formula regression needs an existing amount of 30.'
    $totalsBefore = $grid.Range('O3:BA3').Formula
    $before = V-View $window $grid
    $beforeFormulaOperations = (V-Server).counts.operations
    V-Assert ([bool]$vpApp.EnableEvents) 'The formula test must use the real SheetChange event.'
    $formulaArguments = [object[]]::new(1)
    $formulaArguments[0] = '=10+20'
    # ReadField rejects formula input with its existing native validation
    # dialog. The root observes/dismisses that test-only message while this
    # COM setter waits, then verifies the rejected formula was really removed.
    $vpReport.expected_native_dialog = 'Enter a value, not a formula'
    V-Phase 'awaiting_formula_validation_dialog'
    [void]$amountCell.GetType().InvokeMember('Formula',[Reflection.BindingFlags]::SetProperty,
        $null,$amountCell,$formulaArguments,[Globalization.CultureInfo]::GetCultureInfo('en-US'))
    $vpReport.expected_native_dialog = $null
    V-Phase 'formula_validation_dialog_dismissed'
    V-Assert (-not [bool]$amountCell.HasFormula) 'An equal-result numeric formula survived the SheetChange render.'
    V-Assert ([double]$amountCell.Value2 -eq 30) 'Literalizing the entered formula changed its value.'
    V-ExactView $before (V-View $window $grid) 'Equal-value formula capture'
    V-Sync
    V-Assert (-not [bool]$amountCell.HasFormula -and [double]$amountCell.Value2 -eq 30) 'Sync retained an untracked numeric formula.'
    V-Assert ((V-Server).counts.operations -eq $beforeFormulaOperations) 'An equal-value formula echoed an unnecessary mutation.'
    $totalsAfter = $grid.Range('O3:BA3').Formula
    $totalFormulaCount = 0
    for ($column=1; $column -le $totalsBefore.GetLength(1); $column++) {
        $beforeFormula = [string]$totalsBefore.GetValue(1,$column)
        V-Assert ($beforeFormula -ceq [string]$totalsAfter.GetValue(1,$column)) 'The literal-only receipt grid changed a row-3 total formula.'
        if ($beforeFormula.StartsWith('=')) { $totalFormulaCount++ }
    }
    V-Assert ($totalFormulaCount -gt 0) 'The totals preservation test did not inspect any formulas.'
    $vpReport.checks += 'A rejected =10+20 formula over existing CREDIT 30 is removed and baseline literal 30 restored, without POST; every row-3 total formula remains unchanged'

    V-Phase 'independent_multiarea_anchors_after_middle_deletion'
    $firstAreaRow = Get-ExcelSyncTestRow $vpBook '50'
    $secondAreaRow = Get-ExcelSyncTestRow $vpBook '90'
    $deleteBetweenRow = Get-ExcelSyncTestRow $vpBook '70'
    V-Assert ($firstAreaRow+1 -lt $deleteBetweenRow -and $deleteBetweenRow -lt $secondAreaRow) 'The deletion must be strictly between both selected areas.'
    $firstArea = $grid.Range('AH' + $firstAreaRow + ':AI' + ($firstAreaRow+1))
    $secondArea = $grid.Range('AH' + $secondAreaRow + ':AI' + ($secondAreaRow+1))
    $multiArea = $vpApp.Union($firstArea,$secondArea)
    $multiArea.Select()
    $grid.Range('AI' + ($secondAreaRow+1)).Activate()
    $window.ScrollRow = 110
    $window.ScrollColumn = 35
    V-Assert ([int]$window.RangeSelection.Areas.Count -eq 2) 'The regression must exercise two separate selected areas.'
    $before = V-View $window $grid
    $removed = Invoke-ExcelSyncFixtureControl $vpSession DELETE '/__test__/record/70'
    V-Assert ($removed.status -ceq 'deleted') 'Could not delete the synthetic row between the selected areas.'
    $vpExtraDeleted++
    V-Sync
    $after = V-View $window $grid
    $vpReport.snapshots += [ordered]@{case='multiarea_delete_between';before=$before;after=$after}
    V-AnchorView $before $after 'Deletion between separate selected areas'
    V-Assert ([int]$window.RangeSelection.Areas.Count -eq 2) 'The refresh collapsed the two selected areas.'
    $areaOneAfter = $window.RangeSelection.Areas.Item(1)
    $areaTwoAfter = $window.RangeSelection.Areas.Item(2)
    V-Assert ([int]$areaOneAfter.Row -eq $firstAreaRow) 'The first selected area moved although the deletion was below it.'
    V-Assert ([int]$areaTwoAfter.Row -eq $secondAreaRow-1) 'The second selected area did not move with its own receipts.'
    V-Assert ([int]$after.scroll_row -eq [int]$before.scroll_row-1) 'The viewport lost its top receipt after deletion between selected areas.'
    $vpReport.checks += 'Two disjoint selections on opposite sides of a deletion keep their own UUID ranges; only the later area moves, including its inner active cell'

    V-Phase 'inactive_structural_refresh_preserves_selected_receipt'
    $selectedRow = Get-ExcelSyncTestRow $vpBook '100'
    $grid.Range('AH' + $selectedRow + ':AI' + ($selectedRow+1)).Select()
    $grid.Range('AI' + ($selectedRow+1)).Activate()
    $window.ScrollRow = 160
    $window.ScrollColumn = 35
    $targetBefore = V-View $window $grid
    V-Assert ((Get-ExcelSyncTestRow $vpBook '10') -lt $selectedRow) 'The background deletion must be above the selected receipt.'
    V-Assert ($null -ne $vpOtherBook) 'Expected the runner-owned second workbook.'
    $vpOtherBook.Activate()
    $otherWindow = $vpOtherBook.Windows.Item(1)
    $otherSheet = $vpOtherBook.Worksheets.Item(1)
    $otherBefore = V-View $otherWindow $otherSheet
    $otherName = [string]$vpOtherBook.Name
    $removed = Invoke-ExcelSyncFixtureControl $vpSession DELETE '/__test__/record/10'
    V-Assert ($removed.status -ceq 'deleted') 'Could not delete the synthetic row above the inactive selection.'
    $vpExtraDeleted++
    V-Sync
    V-Assert ([string]$vpApp.ActiveWorkbook.Name -ceq $otherName) 'A structural background refresh stole focus.'
    V-ExactView $otherBefore (V-View $otherWindow $otherSheet) 'Other workbook during background deletion'
    V-Assert ([bool]$grid.Cells.Item($selectedRow,34).Locked) 'The deferred old grid must be locked until its identities are reconciled.'
    # Workbook_Activate must apply the already-received structural change
    # before editing resumes; a later HTTP refresh cannot repair a wrong edit.
    $vpBook.Activate()
    $targetAfter = V-View $window $grid
    $vpReport.snapshots += [ordered]@{case='inactive_delete_above';before=$targetBefore;after=$targetAfter}
    V-AnchorView $targetBefore $targetAfter 'Returning after an inactive structural refresh'
    V-Assert ([int]$targetAfter.scroll_row -eq [int]$targetBefore.scroll_row-1) 'The inactive window lost its viewport anchor.'
    V-Assert ([int]$window.ActiveCell.Row -eq $selectedRow) 'The active receipt did not move up by exactly one row on return.'
    V-Assert (-not [bool]$grid.Cells.Item($selectedRow,34).Locked) 'Workbook activation did not restore safe editing after reconciliation.'
    V-Sync
    V-ExactView $targetAfter (V-View $window $grid) 'Normal tick after deferred structural reconciliation'
    $vpReport.checks += 'A deletion above an inactive selection never steals focus; returning preserves the selected receipt UUIDs, inner active cell and viewport anchor'

    V-Phase 'final_reconcile'
    V-EmptyQueue
    V-CheckGrid
    $server = V-Server
    $local = V-State
    V-Assert (@($server.records).Count -eq ($vpReport.seed_count-1-$vpExtraDeleted)) 'Unexpected final receipt count.'
    V-Assert (@($local.records.PSObject.Properties).Count -eq @($server.records).Count) 'Server and local record counts differ.'
    $last = [int]$grid.Cells.Item($grid.Rows.Count,52).End(-4162).Row
    $identities = $grid.Range('AY5:BA' + $last).Value2
    $identityById = @{}
    for ($i=1; $i -le $identities.GetLength(0); $i++) {
        $id = [string]$identities.GetValue($i,1)
        V-Assert (-not $identityById.ContainsKey($id)) 'The final grid contains a duplicate receipt ID.'
        $identityById[$id] = @([string]$identities.GetValue($i,2),[string]$identities.GetValue($i,3))
    }
    foreach ($record in $server.records) {
        $identity = $identityById[[string]$record.id]
        V-Assert ($null -ne $identity -and $identity[0] -ceq [string]$record.uuid) 'Final grid UUID differs from the server.'
        V-Assert ($identity[1] -ceq [string]$record.version) 'Final grid version differs from the server.'
    }
    V-Assert ([bool]$vpBook.Saved) 'The final synchronized view was not saved.'
    $vpReport.final_records = @($server.records).Count
    $vpReport.passed = $true
    V-Phase 'complete'
} catch {
    $vpReport.error = $_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'
    $vpReport.error_line = $_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    if ($vpOwnClipboard) { $vpApp.CutCopyMode = $false }
    if ($null -ne $vpBook -and $null -ne $vpSession) {
        try { [void](Invoke-ExcelSyncMacro $vpBook 'ExcelSync.StopSync') }
        catch { $vpReport.cleanup_error = $_.Exception.Message }
    }
    if ($null -ne $vpOtherBook) {
        try { $vpApp.EnableEvents=$false; $vpOtherBook.Close($false) }
        catch { $vpReport.other_workbook_cleanup_error = $_.Exception.Message }
    }
    $vpApp.EnableEvents = $vpOriginalEvents
    $vpApp.ScreenUpdating = $vpOriginalScreenUpdating
    $vpReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $vpReportPath -Encoding UTF8
}
[pscustomobject]$vpReport
