param([Parameter(Mandatory)]$Workbook,[Parameter(Mandatory)]$Session)
# Caller owns Excel. Only this synthetic book is reopened; the one disposable
# foreign book created here is the only extra book this script may close.
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if ($Session -is [string]) { $Session=Get-Content -LiteralPath $Session -Raw -Encoding UTF8 | ConvertFrom-Json }
$neBook=$Workbook; $neApp=$Workbook.Application; $neEvents=$neApp.EnableEvents; $neScreen=$neApp.ScreenUpdating
$neValid=$false; $neCopy=$false; $neClosed=$false; $neForeign=$null; $nePath=$null
$neReport=[ordered]@{native=$true;passed=$false;phase='validate';workbook_id=$Session.workbook_id;checks=@()}
function E-Assert([bool]$Value,[string]$Text) { if (-not $Value) { throw $Text } }
function E-Phase([string]$Text) { $script:neReport.phase=$Text; $script:neReport | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $script:nePath -Encoding UTF8 }
function E-Server { return Invoke-ExcelSyncFixtureControl $Session GET '/__test__/state' }
function E-State { return Read-ExcelSyncTestState $Session }
function E-Fault([hashtable]$Fields) { [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/fault' $Fields) }
function E-Sync { [void](Invoke-ExcelSyncTestSync $neBook $Session) }
function E-Col([string]$Bank,[int]$Core) { if ($Bank -cne 'HUMO Card' -or $Core -le 25) { return $Core }; if ($Core -eq 26) { return 32 }; if ($Core -eq 32) { return 33 }; return $Core+7 }
function E-First([string]$Bank) { if ($Bank -ceq 'HUMO Card') { return 4 }; return 5 }
function E-Sheet([string]$Bank) { return $neBook.Worksheets.Item($Bank) }
function E-Activate([string]$Bank) { $neBook.Activate(); (E-Sheet $Bank).Activate(); [void](Invoke-ExcelSyncMacro $neBook 'ExcelSync.ResumeView') }
function E-Last([string]$Bank) { $sheet=E-Sheet $Bank; return [int]$sheet.Cells.Item($sheet.Rows.Count,(E-Col $Bank 52)).End(-4162).Row }
function E-Order([string]$Bank) { $sheet=E-Sheet $Bank; $keys=@(); for ($row=(E-First $Bank); $row -le (E-Last $Bank); $row++) { $keys+=[string]$sheet.Cells.Item($row,(E-Col $Bank 52)).Value2 }; return ,$keys }
function E-View([string]$Bank) {
    $sheet=E-Sheet $Bank; $window=$neBook.Windows.Item(1); $panes=@()
    E-Assert ([string]$window.ActiveSheet.Name -ceq $Bank) 'View capture requires the named bank to be active.'
    for ($i=1; $i -le [int]$window.Panes.Count; $i++) { $pane=$window.Panes.Item($i); $panes+=[ordered]@{row=[int]$pane.ScrollRow;column=[int]$pane.ScrollColumn} }
    return [pscustomobject][ordered]@{bank=$Bank;active=[string]$window.ActiveCell.Address($false,$false);active_key=[string]$sheet.Cells.Item([int]$window.ActiveCell.Row,(E-Col $Bank 52)).Value2;
        selection=[string]$window.RangeSelection.Address($false,$false);scroll_row=[int]$window.ScrollRow;scroll_column=[int]$window.ScrollColumn;
        zoom=[int]$window.Zoom;freeze=[bool]$window.FreezePanes;split_row=[double]$window.SplitRow;split_column=[double]$window.SplitColumn;panes=$panes}
}
function E-Exact($Before,$After,[string]$Context) { E-Assert (($Before | ConvertTo-Json -Depth 10 -Compress) -ceq ($After | ConvertTo-Json -Depth 10 -Compress)) ($Context+': viewport or active receipt changed.') }
function E-Empty { $state=E-State; foreach ($name in @('intents','outbox','blocked')) { E-Assert (@($state.$name.PSObject.Properties).Count -eq 0) ('Expected empty '+$name) } }
function E-Put([string]$Bank,[int]$Row,[int]$Core,$Value) {
    $cell=(E-Sheet $Bank).Cells.Item($Row,(E-Col $Bank $Core)); E-Assert (-not [bool]$cell.Locked) 'Native bank input is locked.'
    $put=[object[]]::new(1); $put[0]=$Value
    [void]$cell.GetType().InvokeMember('Value2',[Reflection.BindingFlags]::SetProperty,$null,$cell,$put,[Globalization.CultureInfo]::GetCultureInfo('en-US'))
}
function E-Requests($Server) { return @($Server.requests | ForEach-Object { [string]$_.method+' '+[string]$_.path+' '+[string]$_.status }) }
try {
    E-Assert ([string]$Session.test_marker -ceq 'TBSparcer isolated native Excel test' -and [string]$Session.api_url -ceq 'http://127.0.0.1:8767') 'Only the isolated native fixture is allowed.'
    $root=[IO.Path]::GetFullPath([string]$Session.test_root).TrimEnd('\','/'); $path=[IO.Path]::GetFullPath([string]$neBook.FullName)
    E-Assert ($path.StartsWith($root+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and $path -ceq [IO.Path]::GetFullPath([string]$Session.workbook_path)) 'Workbook/session must identify an owned test copy.'
    E-Assert (-not [bool]$neBook.ReadOnly -and [IO.Path]::GetExtension($path) -ceq '.xlsm') 'Expected a writable synthetic XLSM.'
    $cfg=$neBook.Worksheets.Item('_sync')
    E-Assert ([string]$cfg.Range('B1').Value2 -ceq [string]$Session.workbook_id -and [string]$cfg.Range('B5').Value2 -ceq [string]$Session.api_url) 'Workbook connection differs from the synthetic session.'
    E-Assert ([string](E-State).server_url -ceq [string]$Session.api_url) 'The durable client state contains a different server origin.'
    E-Assert ([int]$neApp.CutCopyMode -eq 0) 'Finish the current copy/cut before this owned clipboard test.'
    $server=E-Server
    E-Assert ($server.synthetic -and [string]$server.instance_id -ceq [string]$Session.instance_id -and -not $server.faults.offline) 'Wrong or offline synthetic fixture.'
    E-Assert (-not $server.faults.lose_next_sync_ack -and [int]$server.faults.fail_next_sync -eq 0 -and [int]$server.faults.fail_next_snapshot -eq 0) 'Existing fixture faults must be clear.'
    $neValid=$true; $nePath=Join-Path $root 'native-bank-edges-verification.json'; $otherBooks=@()
    foreach ($other in $neApp.Workbooks) { if ([string]$other.FullName -cne $path) { $otherBooks+=@{path=[string]$other.FullName;saved=[bool]$other.Saved} } }
    $neApp.EnableEvents=$true; $neApp.ScreenUpdating=$true
    [void](Invoke-ExcelSyncMacro $neBook 'ExcelSync.StartSync'); E-Activate 'CardXabar'; E-Sync; E-Empty
    E-Assert ((E-Last 'CardXabar') -ge 6 -and (E-Last 'HUMO Card') -ge 4) 'Both bank views need synthetic rows; CardXabar needs at least two.'

    E-Phase 'clipboard_defers_all_http_and_preserves_view'
    $sheet=E-Sheet 'CardXabar'; $sheet.Range($sheet.Cells.Item(5,37),$sheet.Cells.Item(6,38)).Select(); $sheet.Cells.Item(6,38).Activate()
    $id=[string]$sheet.Cells.Item(5,51).Value2; $old=[string]$sheet.Cells.Item(5,37).Value2; $marker='native-bank-copy-'+[guid]::NewGuid().ToString('N')
    [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/server-edit' @{id=[int]$id;fields=@{annotation_details=$marker}})
    $before=E-View 'CardXabar'; $requestBefore=E-Requests (E-Server)
    $neBook.Windows.Item(1).RangeSelection.Copy(); $neCopy=$true
    E-Assert ([int]$neApp.CutCopyMode -eq 1) 'The owned native copy did not enter copy mode.'
    E-Sync
    E-Assert ([int]$neApp.CutCopyMode -eq 1 -and [string]$sheet.Cells.Item(5,37).Value2 -ceq $old) 'Sync cancelled copy mode or changed copied cells.'
    E-Exact $before (E-View 'CardXabar') 'Clipboard deferral'
    # The log retains only 100 requests. Its only new entry may be our preceding
    # inspector GET; comparing the bounded suffix detects any actual HTTP call.
    $expectedRequests=@(($requestBefore+@('GET /__test__/state 200')) | Select-Object -Last 100)
    E-Assert (((E-Requests (E-Server))-join '|') -ceq ($expectedRequests-join '|')) 'Sync sent HTTP while Excel copy mode was active.'
    $neApp.CutCopyMode=$false; $neCopy=$false; E-Sync
    E-Assert ([string]$sheet.Cells.Item(5,37).Value2 -ceq $marker) 'Ending the owned copy did not allow the pending snapshot to render.'
    E-Exact $before (E-View 'CardXabar') 'Clipboard release and resumed sync'
    $neReport.checks+='Native copy blocks all sync HTTP and cell changes, preserves clipboard/view, and resumes after ending only the owned copy mode'

    E-Phase 'humo_inactive_create_ack_then_close_reopen'
    E-Activate 'HUMO Card'; E-Empty; $sheet=E-Sheet 'HUMO Card'; $row=(E-Last 'HUMO Card')+1; $marker='native-bank-deferred-ack-'+[guid]::NewGuid().ToString('N')
    $counts=(E-Server).counts; E-Fault @{offline=$true}
    E-Put 'HUMO Card' $row 21 $marker
    E-Put 'HUMO Card' $row 17 ([datetime]'2001-01-01T12:34:56').ToOADate()
    E-Put 'HUMO Card' $row 20 'DEBIT'; E-Put 'HUMO Card' $row 34 '0063'; E-Put 'HUMO Card' $row 41 'USD'; E-Put 'HUMO Card' $row 26 ([double]63.27)
    $uuid=[string]$sheet.Cells.Item($row,59).Value2; $parsed=[guid]::Empty
    E-Assert ([guid]::TryParse($uuid,[ref]$parsed)) 'Native HUMO draft has no UUID.'
    $sheet.Range($sheet.Cells.Item($row,44),$sheet.Cells.Item($row,45)).Select(); $sheet.Cells.Item($row,45).Activate()
    $before=E-View 'HUMO Card'; $order=E-Order 'HUMO Card'
    E-Activate 'CardXabar'; E-Fault @{offline=$false}; E-Sync; E-Empty
    $server=E-Server; $created=@($server.records | Where-Object {[string]$_.fields.operator_raw -ceq $marker})
    E-Assert ($created.Count -eq 1 -and [int]$server.counts.transactions -eq [int]$counts.transactions+1 -and [int]$server.counts.operations -eq [int]$counts.operations+1) 'Inactive HUMO draft was not committed exactly once.'
    $serverUuid=[string]$created[0].uuid
    E-Assert ($serverUuid -cne $uuid -and [string]$created[0].fields.excel_bank -ceq 'HUMO Card') 'Expected a new server UUID and implicit HUMO assignment.'
    E-Assert (((E-Order 'HUMO Card')-join ',') -ceq ($order-join ',') -and [bool]$sheet.Cells.Item($row,44).Locked) 'Inactive HUMO draft was moved or remained editable before activation.'
    E-Assert ([string]$neBook.Windows.Item(1).ActiveSheet.Name -ceq 'CardXabar') 'The inactive ACK activated HUMO.'
    $neBook.Save(); $neBook.Close($true); $neBook=$null; $neClosed=$true
    $neBook=$neApp.Workbooks.Open($path,0,$false); $neClosed=$false
    E-Assert ([string]$neBook.Windows.Item(1).ActiveSheet.Name -ceq 'CardXabar') 'Reopen activated HUMO before its deferred view was inspected.'
    $resolved=[string](Invoke-ExcelSyncMacro $neBook 'ExcelSync.ViewDisplayKey' @('u:'+$uuid))
    E-Assert ($resolved -ceq ('u:'+$serverUuid)) 'Reopened durable cache lost the draft-to-server UUID alias.'
    E-Assert (((E-Order 'HUMO Card')-join ',') -ceq ($order-join ',')) 'Reopen rebuilt inactive HUMO before activation.'
    E-Activate 'HUMO Card'; $expectedOrder=@($order | ForEach-Object { if ($_ -ceq $uuid) { $serverUuid } else { $_ } }); $before.active_key=$serverUuid
    E-Exact $before (E-View 'HUMO Card') 'Deferred ACK after actual reopen'
    E-Assert (((E-Order 'HUMO Card')-join ',') -ceq ($expectedOrder-join ',')) 'Activation did not replace the draft UUID in its original bank row.'
    $neReport.draft=@{local_uuid=$uuid;server_uuid=$serverUuid;server_id=[string]$created[0].id;row=$row}
    $neReport.checks+='HUMO create ACK while CardXabar is active survives actual close/reopen; durable alias preserves draft order, selection and active receipt on first HUMO activation'

    E-Phase 'explicit_bank_sort_keeps_selected_uuid'
    foreach ($bank in @('HUMO Card','CardXabar')) {
        E-Activate $bank; $sheet=E-Sheet $bank; $selectedRow=E-Last $bank; $sheet.Cells.Item($selectedRow,(E-Col $bank 37)).Select()
        $selected=[string]$sheet.Cells.Item($selectedRow,(E-Col $bank 52)).Value2
        [void](Invoke-ExcelSyncMacro $neBook 'BankViews.SortByDate')
        $view=E-View $bank; E-Assert ($view.active_key -ceq $selected) 'Explicit bank sort detached the selected receipt UUID.'
        $previous=[double]::NegativeInfinity
        for ($r=(E-First $bank); $r -le (E-Last $bank); $r++) { $date=[double]$sheet.Cells.Item($r,17).Value2; E-Assert ($date -ge $previous) 'Bank sort did not order dates ascending.'; $previous=$date }
        if ($bank -ceq 'HUMO Card') { E-Assert ([string]$sheet.Cells.Item(4,59).Value2 -ceq $serverUuid) 'The 2001 draft did not move from the bottom to the first HUMO row.' }
    }
    E-Phase 'foreign_same_name_sheet_sort_is_untouched'
    $ownedCard=E-Order 'CardXabar'; $ownedHumo=E-Order 'HUMO Card'; $neForeign=$neApp.Workbooks.Add(-4167); $foreign=$neForeign.Worksheets.Item(1); $foreign.Name='CardXabar'
    for ($r=5; $r -le 8; $r++) { $foreign.Cells.Item($r,17).Value2=[double](40009-$r); $foreign.Cells.Item($r,21).Value2='foreign-'+$r; $foreign.Cells.Item($r,52).Value2='foreign-uuid-'+$r }
    $foreign.Range('U6:V7').Select(); $foreignBefore=$foreign.Range('O5:BA8').Value2 | ConvertTo-Json -Depth 10 -Compress
    $foreignName=[string]$neForeign.Name; $foreignSelection=[string]$neForeign.Windows.Item(1).RangeSelection.Address($false,$false)
    [void](Invoke-ExcelSyncMacro $neBook 'BankViews.SortByDate')
    E-Assert ([string]$neApp.ActiveWorkbook.Name -ceq $foreignName -and [string]$neApp.ActiveSheet.Name -ceq 'CardXabar') 'Owned bank sort stole focus from the foreign workbook.'
    E-Assert (($foreign.Range('O5:BA8').Value2 | ConvertTo-Json -Depth 10 -Compress) -ceq $foreignBefore -and [string]$neForeign.Windows.Item(1).RangeSelection.Address($false,$false) -ceq $foreignSelection) 'Owned sort changed foreign cells/order or selection.'
    E-Assert (((E-Order 'CardXabar')-join ',') -ceq ($ownedCard-join ',') -and ((E-Order 'HUMO Card')-join ',') -ceq ($ownedHumo-join ',')) 'Foreign invocation changed the owned bank order.'
    $neForeign.Close($false); $neForeign=$null
    foreach ($other in $otherBooks) { $matches=@($neApp.Workbooks | Where-Object {[string]$_.FullName -ceq [string]$other.path}); E-Assert ($matches.Count -eq 1 -and [bool]$matches[0].Saved -eq [bool]$other.saved) 'An unrelated workbook was closed or dirtied.' }
    $neReport.checks+='Explicit SortByDate preserves selected UUID on both banks; invoking it with an owned foreign CardXabar sheet active changes neither workbook and does not steal focus'
    E-Empty; $neReport.reopened_workbook_name=[string]$neBook.Name; $neReport.passed=$true; E-Phase 'complete'
} catch { $neReport.error=$_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'; $neReport.error_line=$_.InvocationInfo.ScriptLineNumber; throw }
finally {
    if ($neCopy) { $neApp.CutCopyMode=$false }
    if ($neValid) {
        try { E-Fault @{offline=$false} } catch { $neReport.fixture_cleanup_error=$_.Exception.Message }
        if ($neClosed -and $null -eq $neBook) { try { $neApp.EnableEvents=$true; $neBook=$neApp.Workbooks.Open([string]$Session.workbook_path,0,$false) } catch { $neReport.reopen_error=$_.Exception.Message } }
        if ($null -ne $neBook) { try { [void](Invoke-ExcelSyncMacro $neBook 'ExcelSync.StopSync') } catch { $neReport.stop_error=$_.Exception.Message } }
    }
    if ($null -ne $neForeign) { try { $neForeign.Close($false) } catch { $neReport.foreign_cleanup_error=$_.Exception.Message } }
    $neApp.EnableEvents=$neEvents; $neApp.ScreenUpdating=$neScreen
    if ($nePath) { $neReport | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $nePath -Encoding UTF8 }
}
[pscustomobject]$neReport
