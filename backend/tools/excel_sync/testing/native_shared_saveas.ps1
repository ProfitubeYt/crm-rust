param([Parameter(Mandatory)]$Workbook,[Parameter(Mandatory)]$Session)
# Run after the caller's native AX conflict-resolution/delete UI verification.
# Owns no Excel lifecycle except closing this already-saved synthetic copy at end.
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if($Session -is [string]){$Session=Get-Content -LiteralPath $Session -Raw -Encoding UTF8|ConvertFrom-Json}
$app=$Workbook.Application
$book=$Workbook
$oldEvents=$app.EnableEvents
$reportPath=Join-Path $Session.test_root 'native-saveas-timer-verification.json'
$report=[ordered]@{native=$true;passed=$false;phase='verify_row_actions';workbook_id=$Session.workbook_id;manual_sync_calls=0}
function Assert([bool]$Value,[string]$Message){if(-not $Value){throw $Message}}
function Phase([string]$Value){$script:report.phase=$Value;$script:report|ConvertTo-Json -Depth 10|Set-Content -LiteralPath $script:reportPath -Encoding UTF8}
function State{return Read-ExcelSyncTestState $script:Session}
function Server{return Invoke-ExcelSyncFixtureControl $script:Session GET '/__test__/state'}
function EmptyQueue{foreach($key in @('intents','outbox','blocked')){Assert (@((State).$key.PSObject.Properties).Count -eq 0) ('Expected empty '+$key)}}
try {
    Assert ($Session.test_marker -ceq 'TBSparcer isolated native Excel test') 'Expected an isolated test session.'
    Assert ([string]$book.FullName -ceq [string]$Session.workbook_path) 'Workbook/session path mismatch.'
    Assert ([string]$book.Worksheets.Item('_sync').Range('B1').Value2 -ceq [string]$Session.workbook_id) 'Workbook identity mismatch.'
    $server=Server
    Assert ($server.synthetic -and $server.instance_id -ceq $Session.instance_id) 'Unexpected synthetic API instance.'
    Assert (-not $server.faults.offline) 'Fixture must be online.'
    EmptyQueue
    $state=State
    $serverIds=@($server.records.id|Sort-Object)
    $localIds=@($state.records.PSObject.Properties.Value.id|Sort-Object)
    Assert (($serverIds -join ',') -ceq '1,2,3' -and ($localIds -join ',') -ceq '1,2,3') 'Expected retained receipts1/2/3 and deleted receipt4.'
    $resolved=@($server.records|Where-Object {[string]$_.id -ceq '3'})[0]
    $local=@($state.records.PSObject.Properties.Value|Where-Object {[string]$_.id -ceq '3'})[0]
    Assert ([string]$resolved.fields.annotation_details -match '^shared-local-conflict-[0-9a-f]{32}$') 'The keep-mine conflict action did not retain the local proposal.'
    Assert ([string]$resolved.fields.annotation_details -ceq [string]$local.fields.annotation_details) 'Server/state conflict outcome differs.'
    $operations=@($server.operations|Where-Object {[string]$_.workbook_id -ceq [string]$Session.workbook_id})
    $lastStatuses=@($operations|Select-Object -Last 3|ForEach-Object {$_.status})
    Assert (($lastStatuses -join ',') -ceq 'conflict,applied,deleted') 'Expected resolved conflict then confirmed deletion in the ledger.'
    $actions=[ordered]@{native=$true;passed=$true;workbook_id=$Session.workbook_id;ui_observed_by_parent=$true;
        resolved_id='3';deleted_id='4';server_ids=$serverIds;state_ids=$localIds;intents=0;outbox=0;blocked=0;
        keep_mine_value_matches_server_and_state=$true;last_ledger_statuses=$lastStatuses}
    $actions|ConvertTo-Json -Depth 10|Set-Content -LiteralPath (Join-Path $Session.test_root 'native-row-actions-verification.json') -Encoding UTF8
    Phase 'save_as'
    $oldPath=[string]$book.FullName
    $newPath=Join-Path $Session.test_root 'shared-smoke-saved-as.xlsm'
    Assert (-not(Test-Path -LiteralPath $newPath)) 'The SaveAs destination already exists.'
    $app.EnableEvents=$true
    $book.SaveAs($newPath,52)
    $Session.workbook_path=$newPath
    $Session|ConvertTo-Json -Depth 5|Set-Content -LiteralPath $Session.session_path -Encoding UTF8
    $report.old_workbook_path=$oldPath
    $report.workbook_path=$newPath
    Assert ([string]$book.Worksheets.Item('_sync').Range('B1').Value2 -ceq [string]$Session.workbook_id) 'SaveAs changed the workbook ID.'
    Assert ([bool]$book.Saved) 'SaveAs did not complete.'
    EmptyQueue
    $marker='shared-saveas-timer-'+[guid]::NewGuid().ToString('N')
    [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/server-edit' @{id=3;fields=@{annotation_details=$marker}})
    $before=Server
    $beforeCount=$before.counts.operations
    $snapshotCount=@($before.requests|Where-Object {$_.method -eq 'GET' -and $_.path -eq '/api/excel/v1/snapshot'}).Count
    $row=Get-ExcelSyncTestRow $book '3'
    $grid=$book.Worksheets.Item([string]$book.Worksheets.Item('_sync').Range('B3').Value2)
    $start=[datetime]::UtcNow
    Phase 'waiting_for_real_60_second_ontime'
    $observed=$false
    do {
        Start-Sleep -Milliseconds 500
        try {$observed=([string]$grid.Cells.Item($row,37).Value2 -ceq $marker)}
        catch {
            $code=$_.Exception.HResult
            if($null -ne $_.Exception.InnerException){$code=$_.Exception.InnerException.HResult}
            if($code -notin @(-2147418111,-2147417846)){throw}
        }
    } while(-not $observed -and ([datetime]::UtcNow-$start).TotalSeconds -lt 85)
    $report.wait_seconds=[Math]::Round(([datetime]::UtcNow-$start).TotalSeconds,2)
    Assert $observed 'Native OnTime did not refresh the renamed workbook.'
    $after=Server
    Assert ($after.counts.operations -eq $beforeCount) 'The automatic read refresh echoed an additional mutation.'
    Assert (@($after.requests|Where-Object {$_.method -eq 'GET' -and $_.path -eq '/api/excel/v1/snapshot'}).Count -gt $snapshotCount) 'The automatic timer did not issue a snapshot request.'
    $local=@((State).records.PSObject.Properties.Value|Where-Object {[string]$_.id -ceq '3'})[0]
    Assert ([string]$local.fields.annotation_details -ceq $marker) 'The timer did not persist the new baseline.'
    Assert ([string]$grid.Cells.Item($row,53).Value2 -ceq [string]$local.version) 'Grid version differs from the durable baseline.'
    Assert (@($app.Workbooks|Where-Object {[string]$_.FullName -ceq $oldPath}).Count -eq 0) 'A stale timer reopened the old workbook name.'
    EmptyQueue
    Assert ([bool]$book.Saved) 'The automatic refresh did not save the renamed workbook.'
    Assert ((Get-ExcelSyncTestStatus $book) -ceq [string]$book.Worksheets.Item('_sync').Range('B9').Value2) 'The status mirror differs.'
    $report.passed=$true
    $report.queue_empty=$true
    $report.old_name_reopened=$false
    Phase 'complete'
} catch {
    $report.error=$_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'
    $report.error_line=$_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    if($null -ne $book){
        try {
            [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StopSync')
            if($report.passed){
                $app.EnableEvents=$false
                $book.Close($false)
                $report.test_workbook_closed=$true
            }
        } catch {$report.cleanup_error=$_.Exception.Message}
    }
    $app.EnableEvents=$oldEvents
    $app.UserControl=$true
    $app.Visible=$true
    $report.excel_workbooks_remaining=$app.Workbooks.Count
    $report|ConvertTo-Json -Depth 10|Set-Content -LiteralPath $reportPath -Encoding UTF8
}
[pscustomobject]$report
