param(
    [Parameter(Mandatory)]$Workbook,
    [Parameter(Mandatory)][string]$FixturePath,
    [Parameter(Mandatory)][string]$TestRoot
)
# Run from the caller's existing Excel COM process. Does not open/close Excel.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Entries($Map) { return @($Map.PSObject.Properties | ForEach-Object { $_ }) }
function Server-State { return Invoke-ExcelSyncFixtureControl $session GET '/__test__/state' }
function Client-State { return Read-ExcelSyncTestState $session }
function Sync-Client { return Invoke-ExcelSyncTestSync $Workbook $session }
function Fault([hashtable]$Value) { [void](Invoke-ExcelSyncFixtureControl $session POST '/__test__/fault' $Value) }
function Server-Record([string]$Id) { return @((Server-State).records | Where-Object {[string]$_.id -ceq $Id})[0] }

$verification = [ordered]@{native=$true; workbook=[string]$Workbook.FullName; checks=@(); passed=$false}
$reportPath = Join-Path $TestRoot 'native-e2e-verification.json'
$session = $null
try {
    $session = Initialize-ExcelSyncTestClient -Workbook $Workbook -FixturePath $FixturePath -TestRoot $TestRoot
    $verification.workbook_id = $session.workbook_id
    $verification.session_path = $session.session_path
    Assert-That ((Entries (Client-State).records).Count -eq 2) 'The fresh fixture snapshot must contain two receipts.'
    $verification.checks += 'actual VBA WinCred write/read + authenticated snapshot + owner binding'

    # Use an actual WorksheetChange event and check text formula injection too.
    $literal = '=native-literal-test-' + [guid]::NewGuid().ToString('N')
    [void](Set-ExcelSyncTestFields $Workbook $session '1' @{annotation_details=$literal})
    $sheet = $Workbook.Worksheets.Item([string]$Workbook.Worksheets.Item('_sync').Range('B3').Value2)
    $row = Get-ExcelSyncTestRow $Workbook '1'
    Assert-That (-not $sheet.Cells.Item($row,37).HasFormula) 'Leading equals text became a formula.'
    [void](Sync-Client)
    Assert-That ((Server-Record '1').fields.annotation_details -ceq $literal) 'Worksheet edit did not reach the database exactly.'
    Assert-That ((Entries (Client-State).outbox).Count -eq 0) 'Successful edit left an outbox entry.'
    $verification.checks += 'WorksheetChange -> durable intent -> sync -> exact literal server text'

    # Server commits once but the HTTP acknowledgement is lost.
    $beforeOperations = (Server-State).counts.operations
    Fault @{lose_next_sync_ack=$true}
    [void](Set-ExcelSyncTestFields $Workbook $session '1' @{annotation_details='native update with lost acknowledgement'})
    [void](Sync-Client)
    $pending = Entries (Client-State).outbox
    Assert-That ($pending.Count -eq 1) 'Lost update acknowledgement must retain one immutable operation.'
    $operationId = [string]$pending[0].Name
    $payload = $pending[0].Value | ConvertTo-Json -Depth 50 -Compress
    Assert-That ((Server-State).counts.operations -eq ($beforeOperations + 1)) 'Lost update reply did not commit exactly once.'
    [void](Sync-Client)
    $server = Server-State
    Assert-That ((Entries (Client-State).outbox).Count -eq 0) 'Update retry did not acknowledge its operation.'
    Assert-That ($server.counts.operations -eq ($beforeOperations + 1)) 'Update retry inserted a second ledger operation.'
    Assert-That (@($server.operations | Where-Object {$_.operation_id -eq $operationId}).Count -eq 1) 'Update operation ID was not stable across retry.'
    $verification.checks += 'lost update reply -> same operation ID retry -> one ledger receipt'

    # Temporarily unavailable API with the same in-memory database preserved.
    $offlineText = 'native offline edit retained ' + [guid]::NewGuid().ToString('N')
    Fault @{offline=$true}
    [void](Set-ExcelSyncTestFields $Workbook $session '2' @{annotation_details=$offlineText})
    [void](Sync-Client)
    Assert-That ((Entries (Client-State).intents).Count -eq 1) 'Offline edit was not retained locally.'
    Assert-That ((Server-Record '2').fields.annotation_details -cne $offlineText) 'Offline test unexpectedly wrote to the server.'
    Fault @{offline=$false}
    [void](Sync-Client)
    Assert-That ((Server-Record '2').fields.annotation_details -ceq $offlineText) 'Offline edit was not applied after recovery.'
    $verification.checks += 'offline API -> local persisted edit -> recovery without losing state'

    # Seed only the post-confirmation state; confirmation UI is tested separately.
    $createMarker = 'native-confirmed-create-' + [guid]::NewGuid().ToString('N')
    $beforeCount = (Server-State).counts.transactions
    $beforeOperations = (Server-State).counts.operations
    Fault @{lose_next_sync_ack=$true}
    $draftKey = Add-ExcelSyncConfirmedTestCreate $Workbook $session @{annotation_details=$createMarker}
    [void](Sync-Client)
    $pending = Entries (Client-State).outbox
    Assert-That ($pending.Count -eq 1 -and $pending[0].Value.kind -eq 'create') 'Lost create reply must retain the original create.'
    $createOperationId = [string]$pending[0].Name
    $createPayload = $pending[0].Value | ConvertTo-Json -Depth 50 -Compress
    Assert-That ((Server-State).counts.transactions -eq ($beforeCount + 1)) 'Creation did not commit before its synthetic lost reply.'
    Assert-That ((Entries (Client-State).records).Count -eq ($beforeCount + 1)) 'Lost create reply exposed duplicate local rows.'
    Fault @{fail_next_sync=1}
    [void](Sync-Client)
    $current = Client-State
    Assert-That ((Entries $current.records).Count -eq ($beforeCount + 1)) 'A successful snapshot followed by failed retry exposed duplicate rows.'
    $pending = Entries $current.outbox
    Assert-That ([string]$pending[0].Name -eq $createOperationId) 'Retry changed the create operation ID.'
    Assert-That (($pending[0].Value | ConvertTo-Json -Depth 50 -Compress) -ceq $createPayload) 'Retry changed the immutable create payload.'
    [void](Sync-Client)
    $server = Server-State
    $created = @($server.records | Where-Object {$_.fields.annotation_details -ceq $createMarker})
    Assert-That ($created.Count -eq 1) 'Create retry produced duplicate server receipts.'
    Assert-That ((Entries (Client-State).records).Count -eq ($beforeCount + 1)) 'Acknowledged create did not replace the local draft.'
    Assert-That ((Entries (Client-State).outbox).Count -eq 0) 'Acknowledged create remained in the outbox.'
    Assert-That ($server.counts.operations -eq ($beforeOperations + 1)) 'Create retry wrote more than one ledger operation.'
    $verification.checks += 'confirmed create + lost reply + failed retry snapshot -> no duplicate UI/server receipt'

    $createdId = [string]$created[0].id
    Add-ExcelSyncConfirmedTestDelete $Workbook $session $createdId
    [void](Sync-Client)
    $server = Server-State
    Assert-That (@($server.records | Where-Object {[string]$_.id -eq $createdId}).Count -eq 0) 'Confirmed deletion left the server receipt.'
    Assert-That ($server.counts.transactions -eq $beforeCount) 'Confirmed deletion removed an unexpected number of records.'
    Assert-That ($server.counts.deletions -ge 1) 'Deletion tombstone was not recorded.'
    $verification.checks += 'confirmed deletion engine -> database delete + tombstone + local row removal'

    # Leave a real conflict pending for the caller's authorized UI resolution.
    [void](Invoke-ExcelSyncFixtureControl $session POST '/__test__/server-edit' @{id=1;fields=@{annotation_details='server concurrent change'}})
    [void](Set-ExcelSyncTestFields $Workbook $session '1' @{annotation_details='local concurrent change'})
    [void](Sync-Client)
    $blocked = Entries (Client-State).blocked
    Assert-That (@($blocked | Where-Object {$_.Value.status -eq 'conflict'}).Count -eq 1) 'Concurrent edit did not produce a pending conflict.'
    Assert-That ((Server-Record '1').fields.annotation_details -ceq 'server concurrent change') 'Conflict overwrote server text.'
    $verification.checks += 'concurrent server/client edit -> conflict retained, server value preserved'
    $verification.pending_ui_test = 'Exercise row actions for conflict resolution and confirmed deletion; the shared workbook creates receipts automatically from its bottom input row.'
    $verification.passed = $true
} catch {
    $verification.error = $_.Exception.Message
    throw
} finally {
    if ($null -ne $session) {
        try { Fault @{offline=$false;fail_next_snapshot=0;fail_next_sync=0;lose_next_sync_ack=$false} } catch { }
    }
    $verification | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}
$verification | ConvertTo-Json -Depth 20
