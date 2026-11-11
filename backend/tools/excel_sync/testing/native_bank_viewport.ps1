param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
# Reuses an isolated session and the caller's Excel process. Only this owned
# workbook is closed/reopened. Reacquire it from Application.Workbooks after
# running this script because the original Workbook COM handle is then stale.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if ($Session -is [string]) { $Session=Get-Content -LiteralPath $Session -Raw -Encoding UTF8 | ConvertFrom-Json }
$nvBook=$Workbook
$nvApp=$Workbook.Application
$nvEvents=$nvApp.EnableEvents
$nvScreen=$nvApp.ScreenUpdating
$nvValidated=$false
$nvClosed=$false
$nvReportPath=$null
$nvReport=[ordered]@{native=$true;passed=$false;phase='validate';workbook_id=$Session.workbook_id;checks=@();snapshots=@();reopens=0}

function N-Assert([bool]$Value,[string]$Message) { if (-not $Value) { throw $Message } }
function N-Phase([string]$Value) {
    $script:nvReport.phase=$Value
    $script:nvReport.updated_utc=[datetime]::UtcNow.ToString('o')
    if ($script:nvReportPath) { $script:nvReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $script:nvReportPath -Encoding UTF8 }
}
function N-Server { return Invoke-ExcelSyncFixtureControl $Session GET '/__test__/state' }
function N-State { return Read-ExcelSyncTestState $Session }
function N-Fault([hashtable]$Fields) { [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/fault' $Fields) }
function N-Sync { [void](Invoke-ExcelSyncTestSync $nvBook $Session) }
function N-First([string]$Bank) { if ($Bank -ceq 'HUMO Card') { return 4 }; return 5 }
function N-Col([string]$Bank,[int]$Core) {
    if ($Bank -cne 'HUMO Card' -or $Core -le 25) { return $Core }
    if ($Core -eq 26) { return 32 }
    if ($Core -eq 32) { return 33 }
    if ($Core -ge 33 -and $Core -le 53) { return $Core+7 }
    throw "Unsupported column $Core."
}
function N-Sheet([string]$Bank) { return $nvBook.Worksheets.Item($Bank) }
function N-Activate([string]$Bank) {
    $nvBook.Activate()
    (N-Sheet $Bank).Activate()
    [void](Invoke-ExcelSyncMacro $nvBook 'ExcelSync.ResumeView')
}
function N-Last([string]$Bank) {
    $sheet=N-Sheet $Bank
    return [int]$sheet.Cells.Item($sheet.Rows.Count,(N-Col $Bank 52)).End(-4162).Row
}
function N-Row([string]$Bank,[string]$Id) {
    $sheet=N-Sheet $Bank
    for ($row=(N-First $Bank); $row -le (N-Last $Bank); $row++) {
        if ([string]$sheet.Cells.Item($row,(N-Col $Bank 51)).Value2 -ceq $Id) { return $row }
    }
    throw "Synthetic receipt $Id is missing from $Bank."
}
function N-Key($Sheet,[int]$Row) {
    $bank=[string]$Sheet.Name
    if ($Row -lt (N-First $bank)) { return 'header:'+ $Row }
    $key=[string]$Sheet.Cells.Item($Row,(N-Col $bank 52)).Value2
    if ($key.Length -eq 0) { return 'blank:'+ $Row }
    return $key
}
function N-Order([string]$Bank) {
    $sheet=N-Sheet $Bank
    $keys=@()
    for ($row=(N-First $Bank); $row -le (N-Last $Bank); $row++) { $keys+=N-Key $sheet $row }
    return ,$keys
}
function N-View([string]$Bank) {
    $sheet=N-Sheet $Bank; $window=$nvBook.Windows.Item(1)
    N-Assert ([string]$window.ActiveSheet.Name -ceq $Bank) 'Only capture a bank while its own view is active.'
    $selection=$window.RangeSelection; $active=$window.ActiveCell; $signature=@(); $panes=@()
    for ($i=1; $i -le [int]$selection.Areas.Count; $i++) {
        $area=$selection.Areas.Item($i)
        $signature+=[ordered]@{first_key=(N-Key $sheet ([int]$area.Row));
            last_key=(N-Key $sheet ([int]$area.Row+[int]$area.Rows.Count-1));
            column=[int]$area.Column;rows=[int]$area.Rows.Count;columns=[int]$area.Columns.Count}
    }
    for ($i=1; $i -le [int]$window.Panes.Count; $i++) {
        $pane=$window.Panes.Item($i)
        $panes+=[ordered]@{scroll_row=[int]$pane.ScrollRow;scroll_column=[int]$pane.ScrollColumn;
            top_key=(N-Key $sheet ([int]$pane.ScrollRow))}
    }
    return [pscustomobject][ordered]@{bank=$Bank;active=[string]$active.Address($false,$false);
        active_key=(N-Key $sheet ([int]$active.Row));active_column=[int]$active.Column;
        selection=[string]$selection.Address($false,$false);signature=($signature | ConvertTo-Json -Depth 10 -Compress);
        scroll_row=[int]$window.ScrollRow;scroll_column=[int]$window.ScrollColumn;
        top_key=(N-Key $sheet ([int]$window.ScrollRow));zoom=[int]$window.Zoom;
        freeze=[bool]$window.FreezePanes;split_row=[double]$window.SplitRow;split_column=[double]$window.SplitColumn;panes=$panes}
}
function N-Exact($Before,$After,[string]$Context) {
    foreach ($field in @('bank','active','active_key','active_column','selection','signature','scroll_row','scroll_column','top_key','zoom','freeze','split_row','split_column')) {
        N-Assert ([string]$Before.$field -ceq [string]$After.$field) "${Context}: changed $field."
    }
    N-Assert (($Before.panes | ConvertTo-Json -Compress) -ceq ($After.panes | ConvertTo-Json -Compress)) "${Context}: moved a pane."
}
function N-Anchor($Before,$After,[string]$Context) {
    foreach ($field in @('bank','active_key','active_column','signature','scroll_column','top_key','zoom','freeze','split_row','split_column')) {
        N-Assert ([string]$Before.$field -ceq [string]$After.$field) "${Context}: changed $field."
    }
    N-Assert (@($Before.panes).Count -eq @($After.panes).Count) "${Context}: changed pane count."
    for ($i=0; $i -lt @($Before.panes).Count; $i++) {
        N-Assert ([string]$Before.panes[$i].top_key -ceq [string]$After.panes[$i].top_key -and
            [int]$Before.panes[$i].scroll_column -eq [int]$After.panes[$i].scroll_column) "${Context}: changed a pane anchor."
    }
}
function N-FileHash {
    $stream=$null; $sha=$null
    try {
        $stream=[IO.FileStream]::new([string]$Session.workbook_path,[IO.FileMode]::Open,[IO.FileAccess]::Read,
            ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
        $sha=[Security.Cryptography.SHA256]::Create()
        return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','')
    } finally {
        if ($sha) { $sha.Dispose() }
        if ($stream) { $stream.Dispose() }
    }
}
function N-EmptyQueue {
    $state=N-State
    foreach ($name in @('intents','outbox','blocked')) { N-Assert (@($state.$name.PSObject.Properties).Count -eq 0) "Expected an empty $name queue." }
}
function N-Record([string]$Id) {
    $found=@((N-Server).records | Where-Object {[string]$_.id -ceq $Id})
    N-Assert ($found.Count -eq 1) "Expected one synthetic receipt $Id."
    return $found[0]
}
function N-Set([string]$Bank,[int]$Row,[int]$Core,$Value) {
    $cell=(N-Sheet $Bank).Cells.Item($Row,(N-Col $Bank $Core))
    N-Assert (-not [bool]$cell.Locked) "$Bank R${Row}C${Core} is unexpectedly locked."
    $put=[object[]]::new(1); $put[0]=$Value
    [void]$cell.GetType().InvokeMember('Value2',[Reflection.BindingFlags]::SetProperty,$null,$cell,$put,
        [Globalization.CultureInfo]::GetCultureInfo('en-US'))
}
function N-Hidden([string]$Bank,[int]$Row,[bool]$Value) {
    # Test-only equivalent of the user's manual row visibility command. Restore
    # normal adapter protection; never unlock fields directly in the harness.
    $sheet=N-Sheet $Bank
    $editable=[bool](Invoke-ExcelSyncMacro $nvBook 'ExcelSync.ViewCanEdit')
    $sheet.Unprotect('tbs-layout')
    try { $sheet.Rows.Item($Row).Hidden=$Value }
    finally { [void](Invoke-ExcelSyncMacro $nvBook 'BankViews.SetEditable' @($editable)) }
}
function N-SetupView([string]$Bank) {
    N-Activate $Bank
    $sheet=N-Sheet $Bank; $window=$nvBook.Windows.Item(1); $first=N-First $Bank; $last=N-Last $Bank
    N-Assert ($last-$first+1 -ge 4) 'At least four bank rows are needed for viewport anchors.'
    $window.FreezePanes=$false; $window.SplitRow=[double]($first-1); $window.SplitColumn=14.0; $window.FreezePanes=$true
    $window.Zoom=85
    $left=N-Col $Bank 37; $right=N-Col $Bank 38; $extra=N-Col $Bank 40
    $area1=$sheet.Range($sheet.Cells.Item($last-1,$left),$sheet.Cells.Item($last,$right))
    $area2=$sheet.Range($sheet.Cells.Item($last-1,$extra),$sheet.Cells.Item($last,$extra))
    $selection=$nvApp.Union($area1,$area2)
    $selection.Select()
    $sheet.Cells.Item($last,$right).Activate()
    $window.ScrollRow=[Math]::Max($first+1,$last-2)
    $window.ScrollColumn=$left
    N-Assert ([int]$window.RangeSelection.Areas.Count -eq 2) 'The native multi-area selection was not retained.'
    N-Assert ([bool]$window.FreezePanes -and [int]$window.Panes.Count -gt 1) 'Expected native frozen panes.'
}
function N-DeleteCandidate([string]$Bank,$AssignedIds) {
    $view=N-View $Bank; $sheet=N-Sheet $Bank
    for ($row=(N-First $Bank); $row -lt [int]$view.scroll_row; $row++) {
        $id=[string]$sheet.Cells.Item($row,(N-Col $Bank 51)).Value2
        if ($AssignedIds -contains $id) { return $id }
    }
    throw 'No owned synthetic receipt remains above the viewport anchor.'
}
function N-ReopenOwned {
    $path=[string]$Session.workbook_path
    N-Assert ([string]$nvBook.FullName -ceq $path) 'Only the original owned test workbook may be reopened.'
    $nvBook.Save()
    N-Assert ([bool]$nvBook.Saved) 'The offline test workbook could not be saved before closing.'
    # Actual BeforeClose/Workbook_Open events must run, including durable state
    # persistence and release/reacquisition of this workbook's local lock.
    $nvBook.Close($true)
    $script:nvBook=$null; $script:nvClosed=$true
    $script:nvBook=$nvApp.Workbooks.Open($path,0,$false)
    $script:nvClosed=$false
    $script:nvReport.reopens++
    N-Assert ([string]$nvBook.Worksheets.Item('_sync').Range('B1').Value2 -ceq [string]$Session.workbook_id) 'Reopen changed the workbook identity.'
}

try {
    N-Assert ([string]$Session.test_marker -ceq 'TBSparcer isolated native Excel test' -and [string]$Session.api_url -ceq 'http://127.0.0.1:8767') 'Only the isolated loopback native session is allowed.'
    $root=[IO.Path]::GetFullPath([string]$Session.test_root).TrimEnd('\','/')
    $path=[IO.Path]::GetFullPath([string]$nvBook.FullName)
    N-Assert ($path.StartsWith($root+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) 'Workbook is outside the owned test root.'
    N-Assert ($path -ceq [IO.Path]::GetFullPath([string]$Session.workbook_path) -and -not [bool]$nvBook.ReadOnly -and [IO.Path]::GetExtension($path) -ceq '.xlsm') 'Expected the original writable synthetic XLSM.'
    $config=$nvBook.Worksheets.Item('_sync')
    N-Assert ([string]$config.Range('B1').Value2 -ceq [string]$Session.workbook_id -and [string]$config.Range('B5').Value2 -ceq [string]$Session.api_url) 'Workbook/session connection mismatch.'
    N-Assert ([int]$nvApp.CutCopyMode -eq 0) 'Finish the current copy/cut before running this fixture.'
    $server=N-Server
    N-Assert ($server.synthetic -and [string]$server.instance_id -ceq [string]$Session.instance_id -and -not $server.faults.offline) 'Expected the online synthetic fixture instance.'
    N-Assert (-not $server.faults.lose_next_sync_ack -and [int]$server.faults.fail_next_sync -eq 0 -and [int]$server.faults.fail_next_snapshot -eq 0) 'Clear existing synthetic one-shot faults first.'
    N-Assert (@($server.records).Count -ge 12) 'At least twelve synthetic receipts are required.'
    $nvValidated=$true; $nvReportPath=Join-Path $root 'native-bank-viewport-verification.json'
    $nvReport.workbook_path=$path
    $otherBooks=@()
    foreach ($other in $nvApp.Workbooks) {
        if ([string]$other.FullName -cne $path) { $otherBooks+=@{path=[string]$other.FullName;saved=[bool]$other.Saved} }
    }
    $nvApp.EnableEvents=$true; $nvApp.ScreenUpdating=$true
    [void](Invoke-ExcelSyncMacro $nvBook 'ExcelSync.StartSync')
    N-Activate 'CardXabar'; N-Sync; N-EmptyQueue
    $candidates=@((N-Server).records | Where-Object {[string]$_.id -notin @('2','3')} | Sort-Object {[int]$_.id})
    if ($candidates.Count -lt 12) { $candidates=@((N-Server).records | Sort-Object {[int]$_.id}) }
    $perBank=[Math]::Min(6,[Math]::Floor($candidates.Count/2))
    $assigned=@{'CardXabar'=@();'HUMO Card'=@()}; $banks=@('CardXabar','HUMO Card')
    N-Phase 'prepare_bank_distribution'
    for ($i=0; $i -lt $perBank*2; $i++) {
        $bank=$banks[[int][Math]::Floor($i/$perBank)]; $id=[string]$candidates[$i].id
        $assigned[$bank]+=$id
        [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/server-edit' @{id=[int]$id;fields=@{excel_bank=$bank}})
    }
    $nvReport.assigned_receipts=$assigned
    foreach ($bank in $banks) { N-Activate $bank; N-Sync }
    foreach ($bank in $banks) { N-Activate $bank; N-Sync }
    N-EmptyQueue

    foreach ($bank in $banks) {
        N-Phase ($bank+'_noop_and_manual_hidden_row')
        N-SetupView $bank
        $sheet=N-Sheet $bank; $hiddenRow=(N-First $bank)+1
        N-Hidden $bank $hiddenRow $true
        $nvBook.Save()
        N-Assert ([bool]$nvBook.Saved) 'The prepared view was not saved.'
        $before=N-View $bank; $hash=N-FileHash; $stamp=(Get-Item -LiteralPath $path).LastWriteTimeUtc.Ticks
        $operations=[int](N-Server).counts.operations
        for ($tick=0; $tick -lt 2; $tick++) {
            Start-Sleep -Milliseconds 1100
            N-Sync
            $after=N-View $bank
            N-Exact $before $after 'No-op bank refresh'
            N-Assert ([bool]$sheet.Rows.Item($hiddenRow).Hidden) 'No-op bank render revealed a manually hidden row.'
            N-Assert ([bool]$nvBook.Saved -and (N-FileHash) -ceq $hash) 'No-op bank refresh dirtied or rewrote the saved workbook.'
            N-Assert ((Get-Item -LiteralPath $path).LastWriteTimeUtc.Ticks -eq $stamp) 'No-op bank refresh changed disk write time.'
            $nvReport.snapshots+=@{case=$bank+'_noop_'+$tick;before=$before;after=$after;sha256=$hash}
        }
        N-Assert ([int](N-Server).counts.operations -eq $operations) 'No-op refresh echoed an API mutation.'
        $nvReport.checks+="${bank}: two no-op cycles preserve manual hidden row, multi-area selection, active UUID, all panes, zoom, freeze, Saved, SHA256 and write time"
        N-Hidden $bank $hiddenRow $false
        N-Phase ($bank+'_active_delete_above_selection')
        N-SetupView $bank
        $before=N-View $bank; $deleteId=N-DeleteCandidate $bank $assigned[$bank]
        [void](Invoke-ExcelSyncFixtureControl $Session DELETE ('/__test__/record/'+$deleteId))
        N-Sync
        $after=N-View $bank
        N-Anchor $before $after 'Active bank deletion above selection'
        N-Assert ([int]$after.scroll_row -eq [int]$before.scroll_row-1) 'Deleted row did not compact above the visible UUID anchor.'
        $nvReport.snapshots+=@{case=$bank+'_active_delete';deleted_id=$deleteId;before=$before;after=$after}
        $nvReport.checks+="${bank}: server deletion above selection preserves selected UUIDs, top UUID and every frozen pane anchor"
    }

    foreach ($bank in $banks) {
        N-Phase ($bank+'_inactive_bank_deferred_delete')
        N-SetupView $bank
        $before=N-View $bank; $order=N-Order $bank; $deleteId=N-DeleteCandidate $bank $assigned[$bank]
        $otherBank=if ($bank -ceq 'CardXabar') { 'HUMO Card' } else { 'CardXabar' }
        N-Activate $otherBank
        $otherBefore=N-View $otherBank
        [void](Invoke-ExcelSyncFixtureControl $Session DELETE ('/__test__/record/'+$deleteId))
        N-Sync
        N-Exact $otherBefore (N-View $otherBank) 'Active other bank during deferred deletion'
        N-Assert (((N-Order $bank)-join ',') -ceq ($order-join ',')) 'Inactive bank moved rows before its view could be captured.'
        $target=N-Sheet $bank
        N-Assert ([bool]$target.Cells.Item((N-First $bank),(N-Col $bank 37)).Locked) 'Structurally stale inactive bank remained editable.'
        N-Activate $bank
        $after=N-View $bank
        N-Anchor $before $after 'Activated deferred bank deletion'
        N-Assert (-not [bool]$target.Cells.Item([int]$nvBook.Windows.Item(1).ActiveCell.Row,(N-Col $bank 37)).Locked) 'Activated reconstructed bank stayed locked.'
        N-Assert ((N-Order $bank).Count -eq $order.Count-1) 'Deferred bank deletion did not remove exactly one displayed UUID.'
        $nvReport.snapshots+=@{case=$bank+'_deferred_delete';deleted_id=$deleteId;before=$before;after=$after}
        $nvReport.checks+="${bank}: inactive deletion defers and locks stale rows; activation reconstructs the view around the same selected UUIDs"
    }

    foreach ($bank in $banks) {
        N-Phase ($bank+'_offline_actual_close_reopen')
        N-SetupView $bank
        $sheet=N-Sheet $bank; $row=N-Last $bank; $id=[string]$sheet.Cells.Item($row,(N-Col $bank 51)).Value2
        $key='u:'+[string]$sheet.Cells.Item($row,(N-Col $bank 52)).Value2
        $marker='native-bank-offline-'+[guid]::NewGuid().ToString('N')
        $before=N-View $bank; $serverBefore=(N-Record $id) | ConvertTo-Json -Depth 50 -Compress
        N-Fault @{offline=$true}
        N-Set $bank $row 37 $marker
        N-Sync
        N-Exact $before (N-View $bank) 'Offline bank edit'
        $intent=(N-State).intents.PSObject.Properties[$key]
        N-Assert ($null -ne $intent -and [string]$intent.Value.fields.annotation_details -ceq $marker) 'Offline native edit was not durable.'
        $payload=$intent.Value | ConvertTo-Json -Depth 50 -Compress
        N-Assert (((N-Record $id) | ConvertTo-Json -Depth 50 -Compress) -ceq $serverBefore) 'Offline edit unexpectedly changed the server.'
        N-ReopenOwned
        N-Activate $bank
        N-Exact $before (N-View $bank) 'Actual offline close/reopen'
        N-Assert (((N-State).intents.PSObject.Properties[$key].Value | ConvertTo-Json -Depth 50 -Compress) -ceq $payload) 'Reopen changed or lost the durable edit intent.'
        N-Assert ([string](N-Sheet $bank).Cells.Item((N-Row $bank $id),(N-Col $bank 37)).Value2 -ceq $marker) 'Reopened bank did not render the offline local edit.'
        N-Fault @{offline=$false}
        N-Sync; N-EmptyQueue
        N-Assert ([string](N-Record $id).fields.annotation_details -ceq $marker) 'Recovered connection did not apply the offline edit.'
        N-Exact $before (N-View $bank) 'Offline recovery acknowledgement'
        $nvReport.checks+="${bank}: native offline edit survives actual owned workbook close/reopen, retains its viewport and reaches the server after recovery"
    }

    foreach ($bank in $banks) {
        N-Phase ($bank+'_lost_create_ack_order')
        N-Activate $bank; N-EmptyQueue
        $sheet=N-Sheet $bank; $row=(N-Last $bank)+1; $marker='native-bank-lost-create-'+[guid]::NewGuid().ToString('N')
        $before=N-Server
        N-Fault @{offline=$true}
        N-Set $bank $row 21 $marker
        $uuid=[string]$sheet.Cells.Item($row,(N-Col $bank 52)).Value2
        N-Set $bank $row 17 ([datetime]'2026-09-15T12:34:56').ToOADate()
        N-Set $bank $row 20 'DEBIT'
        N-Set $bank $row 34 '0047'
        N-Set $bank $row 41 'USD'
        N-Set $bank $row 26 ([double]47.89)
        $order=N-Order $bank
        N-Fault @{offline=$false;lose_next_sync_ack=$true}
        N-Sync
        $pending=@((N-State).outbox.PSObject.Properties)
        N-Assert ($pending.Count -eq 1 -and [string]$pending[0].Value.kind -ceq 'create') 'A lost create ACK must retain one immutable create operation.'
        $operationId=[string]$pending[0].Name; $payload=$pending[0].Value | ConvertTo-Json -Depth 50 -Compress
        N-Assert (((N-Order $bank)-join ',') -ceq ($order-join ',')) 'Lost create ACK reordered or duplicated the bank draft.'
        N-Fault @{fail_next_sync=1}
        N-Sync
        $retry=(N-State).outbox.PSObject.Properties[$operationId]
        N-Assert ($null -ne $retry -and ($retry.Value | ConvertTo-Json -Depth 50 -Compress) -ceq $payload) 'Failed retry changed the original create payload or operation ID.'
        N-Assert (((N-Order $bank)-join ',') -ceq ($order-join ',')) 'A pre-retry snapshot exposed duplicate bank rows.'
        N-Sync; N-EmptyQueue
        $after=N-Server
        $created=@($after.records | Where-Object {[string]$_.fields.operator_raw -ceq $marker})
        N-Assert ($created.Count -eq 1 -and [string]$created[0].fields.excel_bank -ceq $bank -and [string]$created[0].fields.amount -ceq '-47.89') 'Lost create retry did not produce exactly one correctly assigned receipt.'
        N-Assert ([int]$after.counts.operations -eq [int]$before.counts.operations+1 -and [int]$after.counts.transactions -eq [int]$before.counts.transactions+1) 'Lost ACK retries committed more than once.'
        $expectedOrder=@($order | ForEach-Object { if ([string]$_ -ceq $uuid) { [string]$created[0].uuid } else { [string]$_ } })
        N-Assert (((N-Order $bank)-join ',') -ceq ($expectedOrder-join ',')) 'Create acknowledgement did not replace the draft UUID in place.'
        $nvReport.checks+="${bank}: lost create ACK and failed retry preserve one immutable operation and bank order; success replaces the local UUID in place exactly once"
    }
    foreach ($other in $otherBooks) {
        $matches=@($nvApp.Workbooks | Where-Object {[string]$_.FullName -ceq [string]$other.path})
        N-Assert ($matches.Count -eq 1 -and [bool]$matches[0].Saved -eq [bool]$other.saved) 'An unrelated workbook was closed or dirtied by the fixture.'
    }
    N-Assert (@($nvApp.Workbooks | Where-Object {[string]$_.FullName -ceq $path}).Count -eq 1) 'Expected exactly one reopened owned workbook.'
    N-Assert ([bool]$nvApp.EnableEvents -and [bool]$nvApp.ScreenUpdating) 'The regression left application event/screen flags disabled.'
    N-Assert ([bool]$nvBook.Saved) 'The final synchronized bank view was not saved.'
    $nvReport.reopened_workbook_name=[string]$nvBook.Name
    $nvReport.passed=$true
    N-Phase 'complete'
} catch {
    $nvReport.error=$_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'
    $nvReport.error_line=$_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    if ($nvValidated) {
        try { N-Fault @{offline=$false;lose_next_sync_ack=$false;fail_next_sync=0;fail_next_snapshot=0} }
        catch { $nvReport.fixture_cleanup_error=$_.Exception.Message }
        if ($nvClosed -and $null -eq $nvBook) {
            try { $nvApp.EnableEvents=$true; $nvBook=$nvApp.Workbooks.Open([string]$Session.workbook_path,0,$false) }
            catch { $nvReport.reopen_cleanup_error=$_.Exception.Message }
        }
        if ($null -ne $nvBook) {
            try { [void](Invoke-ExcelSyncMacro $nvBook 'ExcelSync.StopSync') }
            catch { $nvReport.stop_error=$_.Exception.Message }
        }
    }
    $nvApp.EnableEvents=$nvEvents; $nvApp.ScreenUpdating=$nvScreen
    if ($nvReportPath) { $nvReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $nvReportPath -Encoding UTF8 }
}
[pscustomobject]$nvReport
