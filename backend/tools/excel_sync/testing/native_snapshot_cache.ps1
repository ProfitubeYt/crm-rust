param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
# Synthetic localhost fixture only. Exercise actual Excel events and HTTP cache behavior.
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if($Session -is [string]){$Session=Get-Content -LiteralPath $Session -Raw -Encoding UTF8|ConvertFrom-Json}
function C-Assert([bool]$Value,[string]$Message){if(-not $Value){throw $Message}}
C-Assert ($Session.test_marker -ceq 'TBSparcer isolated native Excel test' -and $Session.api_url -ceq 'http://127.0.0.1:8767') 'Only the synthetic native fixture is allowed.'
$testRoot=[IO.Path]::GetFullPath([string]$Session.test_root).TrimEnd('\','/')
$bookPath=[IO.Path]::GetFullPath([string]$Workbook.FullName)
C-Assert ($bookPath.StartsWith($testRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and $bookPath -ceq [IO.Path]::GetFullPath([string]$Session.workbook_path) -and -not $Workbook.ReadOnly) 'Expected the writable owned fixture.'
[void](Invoke-ExcelSyncTestSync $Workbook $Session)
function C-Server {Invoke-ExcelSyncFixtureControl $Session GET '/__test__/state'}
function C-Sync {[void](Invoke-ExcelSyncTestSync $Workbook $Session)}
function C-Fault($Fields){[void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/fault' $Fields)}
function C-LastGet { @((C-Server).requests|Where-Object {$_.path -eq '/api/excel/v1/snapshot'})[-1] }
$report=[ordered]@{native=$true;passed=$false;checks=@()}
$reportPath=Join-Path (Split-Path $Session.session_path -Parent) 'native-snapshot-cache-verification.json'
try {
 $allName=[string](Invoke-ExcelSyncMacro $Workbook 'BankViews.AllName')
 $sheet=$Workbook.Worksheets.Item($allName)
 $Workbook.Activate();$sheet.Activate()
 C-Sync;C-Sync
 C-Assert ((C-LastGet).status -eq 304) 'Expected authenticated unchanged snapshot.'
 $report.checks+='Warm snapshot uses HTTP304.'
 $before=C-Server
 $id=[string]$sheet.Cells.Item(5,51).Value2
 $prior=@($before.records|Where-Object {[string]$_.id -ceq $id})[0]
 C-Fault @{bad_snapshot_etag=$true}
 $sheet.Cells.Item(5,37).Value2='Synthetic cache guarded edit'
 C-Sync
 $state=Read-ExcelSyncTestState $Session
 $failed=C-Server
 C-Assert ([int]$failed.counts.operations -eq [int]$before.counts.operations) 'Invalid304 must not send queued edits.'
 C-Assert (@($state.intents.PSObject.Properties).Count -eq 1) 'Invalid304 must preserve the intent.'
 C-Assert ((Get-ExcelSyncTestStatus $Workbook) -match 'does not match') 'Expected explicit cache validator failure.'
 $report.checks+='Mismatched304 fails before POST and retains the durable edit.'
 C-Fault @{bad_snapshot_etag=$false}
 C-Sync;C-Sync
 $state=Read-ExcelSyncTestState $Session
 $after=C-Server
 $record=@($after.records|Where-Object {[string]$_.id -ceq $id})[0]
 C-Assert ($record.fields.annotation_details -ceq 'Synthetic cache guarded edit') 'Recovery did not apply the edit.'
 C-Assert ([int]$after.counts.operations -eq [int]$before.counts.operations+1) 'Recovery must apply exactly once.'
 foreach($q in @('intents','outbox','blocked')){C-Assert (@($state.$q.PSObject.Properties).Count -eq 0) ('Expected empty '+$q)}
 $report.checks+='Recovery sends the retained edit exactly once and updates all views.'
 C-Fault @{omit_snapshot_etag=$true}
 C-Sync;C-Assert ((C-LastGet).status -eq 200) 'Legacy server must return200.'
 C-Sync;C-Assert ((C-LastGet).status -eq 200 -and -not (C-LastGet).conditional) 'Missing ETag must disable the conditional request.'
 $report.checks+='Legacy200 without ETag remains compatible.'
 C-Fault @{omit_snapshot_etag=$false}
 C-Sync;C-Sync
 C-Assert ((C-LastGet).status -eq 304) 'Conditional exchange did not recover.'
 $report.checks+='Conditional exchange recovers after an older server response.'
 $report.passed=$true
}catch{$report.error=$_.Exception.Message;throw}finally{
 C-Fault @{bad_snapshot_etag=$false;omit_snapshot_etag=$false}
 $report|ConvertTo-Json -Depth 12|Set-Content -LiteralPath $reportPath -Encoding UTF8
}
[pscustomobject]$report
