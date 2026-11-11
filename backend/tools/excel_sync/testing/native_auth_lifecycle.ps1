# Isolated native Excel authentication and save/close/reopen regression scenario.
# Dot-source in the existing test worker after importing native_fixture.psm1:
#   . (Join-Path $PSScriptRoot 'native_auth_lifecycle.ps1')
# The caller supplies $book, $excel, $session, and $directory. $session is returned
# by Initialize-ExcelSyncTestClient for synthetic owner 1; $directory is its test_root.
# Use a writable XLSM test copy under C:\Dev\tmp\parcer-excel-e2e-* and the isolated
# loopback fixture at http://127.0.0.1:8767. Flush existing operations/conflicts first.
# This script updates the caller's $book when closing and reopening the test copy.
# It creates no Excel instance, changes no trust settings, and uses no real server.
# The JSON result is written to $directory\native-auth-lifecycle-verification.json.
$ErrorActionPreference = 'Stop'

function Assert-NativeLifecycle([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Native-LifecycleEntries($Map) {
    return ,@($Map.PSObject.Properties | ForEach-Object { $_ })
}
function Native-LifecycleServer {
    $state = Invoke-ExcelSyncFixtureControl $session GET '/__test__/state'
    Assert-NativeLifecycle ($state.synthetic -eq $true -and [string]$state.instance_id -ceq [string]$session.instance_id) 'Isolated fixture instance mismatch.'
    return $state
}
function Native-LifecycleClient { return Read-ExcelSyncTestState $session }
function Native-LifecycleFault([hashtable]$Value) {
    [void](Invoke-ExcelSyncFixtureControl $session POST '/__test__/fault' $Value)
}
function Native-LifecycleQueue($State) {
    return ([ordered]@{intents=$State.intents;outbox=$State.outbox;blocked=$State.blocked} | ConvertTo-Json -Depth 100 -Compress)
}
function Native-LifecycleSaveState($State) {
    Assert-NativeLifecycle ([string]$State.workbook_id -ceq [string]$session.workbook_id) 'Refusing to save another workbook state.'
    $expected = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ('TBSparcer\ExcelSync\' + [string]$session.workbook_id + '\state.json')
    Assert-NativeLifecycle ([IO.Path]::GetFullPath([string]$session.state_path) -eq [IO.Path]::GetFullPath($expected)) 'Unexpected test state path.'
    $json = $State | ConvertTo-Json -Depth 100
    [void](Invoke-ExcelSyncMacro $book 'SyncWindows.AtomicWriteUtf8' @([string]$session.state_path, $json))
}
function Native-LifecycleUnpause {
    # Test-only replacement for the state transition after a successful Connect UI.
    # Preserve owner, records and every immutable operation; do not bypass CheckIdentity.
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StopSync')
    $state = Native-LifecycleClient
    $state.paused = $false
    Native-LifecycleSaveState $state
}
function Native-LifecycleUseGrant([string]$CredentialsPath, [string]$ExpectedOwner) {
    $resolved = [IO.Path]::GetFullPath($CredentialsPath)
    $credentialRoot = [IO.Path]::GetDirectoryName($resolved)
    Assert-NativeLifecycle ($credentialRoot -match '^C:\\Dev\\tmp\\parcer-excel-native-api-[0-9a-f]{32}\\state$') 'Credential file is outside an explicit synthetic fixture directory.'
    Assert-NativeLifecycle ([IO.Path]::GetFileName($resolved) -match '^credentials(-[0-9a-f-]+)?\.json$') 'Unexpected synthetic credential filename.'
    $credential = Get-Content -LiteralPath $resolved -Raw -Encoding UTF8 | ConvertFrom-Json
    $roundTrip = $null
    try {
        Assert-NativeLifecycle ($credential.synthetic -eq $true) 'Credential file is not marked synthetic.'
        Assert-NativeLifecycle ([string]$credential.instance_id -ceq [string]$session.instance_id) 'Credential belongs to another fixture instance.'
        Assert-NativeLifecycle ($credential.api_url -eq 'http://127.0.0.1:8767') 'Credential server is not the isolated loopback fixture.'
        Assert-NativeLifecycle ([string]$credential.user_id -ceq $ExpectedOwner) 'Unexpected credential owner.'
        Assert-NativeLifecycle ([string]$credential.token -match '^excel_[A-Za-z0-9_-]{40,}$') 'Malformed synthetic Excel credential.'
        [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StopSync')
        [void](Invoke-ExcelSyncMacro $book 'SyncWindows.SaveCredential' @([string]$session.workbook_id, [string]$credential.token))
        $roundTrip = [string](Invoke-ExcelSyncMacro $book 'SyncWindows.ReadCredential' @([string]$session.workbook_id))
        Assert-NativeLifecycle ($roundTrip -ceq [string]$credential.token) 'Native WinCred did not retain the replacement grant.'
    } finally {
        $roundTrip = $null
        $credential.token = $null
        $credential = $null
    }
}
function Native-LifecycleSync {
    $result = Invoke-ExcelSyncTestSync $book $session
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StopSync')
    return $result
}
function Native-LifecycleRecord($Server, [string]$Id) {
    $matches = @($Server.records | Where-Object { [string]$_.id -ceq $Id })
    Assert-NativeLifecycle ($matches.Count -eq 1) 'Expected one synthetic lifecycle receipt.'
    return $matches[0]
}

$nativeLifecycleReport = [ordered]@{native=$true;passed=$false;checks=@();phase='preconditions'}
$nativeLifecycleSuccess = $false
$nativeLifecycleFixtureValidated = $false
$nativeLifecycleRenewal = $null
$nativeLifecycleClosed = $false
$nativeLifecycleReportPath = Join-Path $directory 'native-auth-lifecycle-verification.json'
$nativeLifecycleOriginalEvents = $excel.EnableEvents
$nativeLifecycleBookPath = [IO.Path]::GetFullPath([string]$book.FullName)
$nativeLifecycleRoot = [IO.Path]::GetFullPath([string]$directory).TrimEnd('\')

try {
    Assert-NativeLifecycle ($null -ne $book -and $null -ne $session) 'The caller must supply an open test workbook and session.'
    Assert-NativeLifecycle ($session.test_marker -eq 'TBSparcer isolated native Excel test') 'Missing native test ownership marker.'
    Assert-NativeLifecycle ($session.api_url -eq 'http://127.0.0.1:8767') 'Only the synthetic loopback fixture is allowed.'
    Assert-NativeLifecycle ($nativeLifecycleRoot.StartsWith('C:\Dev\tmp\parcer-excel-e2e-', [StringComparison]::OrdinalIgnoreCase)) 'Unexpected native test root.'
    Assert-NativeLifecycle ($nativeLifecycleBookPath.StartsWith($nativeLifecycleRoot + '\', [StringComparison]::OrdinalIgnoreCase)) 'Workbook is outside the caller test root.'
    Assert-NativeLifecycle ($nativeLifecycleBookPath -eq [IO.Path]::GetFullPath([string]$session.workbook_path)) 'Workbook/session path mismatch.'
    Assert-NativeLifecycle ($nativeLifecycleRoot -eq [IO.Path]::GetFullPath([string]$session.test_root).TrimEnd('\')) 'Caller/session test root mismatch.'
    Assert-NativeLifecycle ([IO.Path]::GetExtension($nativeLifecycleBookPath) -eq '.xlsm' -and -not $book.ReadOnly) 'A writable XLSM test copy is required.'
    Assert-NativeLifecycle ([string]$book.Worksheets.Item('_sync').Range('B1').Value2 -ceq [string]$session.workbook_id) 'Native workbook ID changed.'
    Assert-NativeLifecycle ([string]$session.owner_id -ceq '1') 'This fixture scenario expects synthetic owner 1.'
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StopSync')
    $nativeLifecycleState = Native-LifecycleClient
    Assert-NativeLifecycle ((Native-LifecycleEntries $nativeLifecycleState.intents).Count -eq 0 -and (Native-LifecycleEntries $nativeLifecycleState.outbox).Count -eq 0 -and (Native-LifecycleEntries $nativeLifecycleState.blocked).Count -eq 0) 'Resolve/flush the earlier UI tests before running lifecycle tests.'
    Assert-NativeLifecycle ([string]$nativeLifecycleState.owner_id -ceq [string]$session.owner_id) 'Local owner does not match the native test session.'
    $nativeLifecycleBaseline = Native-LifecycleServer
    $nativeLifecycleFixtureValidated = $true
    Native-LifecycleFault @{offline=$false;fail_next_snapshot=0;fail_next_sync=0;lose_next_sync_ack=$false}
    $nativeLifecycleReport.workbook_id = [string]$session.workbook_id
    $nativeLifecycleReport.fixture_instance_id = [string]$session.instance_id

    # Lose a create acknowledgement, then renew under the SAME owner and book ID.
    $nativeLifecycleReport.phase = 'same_owner_renewal'
    $nativeLifecycleCreateMarker = 'native-lifecycle-create-' + [guid]::NewGuid().ToString('N')
    Native-LifecycleUnpause
    Native-LifecycleFault @{lose_next_sync_ack=$true}
    [void](Add-ExcelSyncConfirmedTestCreate $book $session @{annotation_details=$nativeLifecycleCreateMarker})
    [void](Native-LifecycleSync)
    $nativeLifecyclePending = Native-LifecycleEntries (Native-LifecycleClient).outbox
    Assert-NativeLifecycle ($nativeLifecyclePending.Count -eq 1 -and $nativeLifecyclePending[0].Value.kind -eq 'create') 'Lost create acknowledgement did not leave exactly one create operation.'
    $nativeLifecycleCreateOperationId = [string]$nativeLifecyclePending[0].Name
    $nativeLifecycleCommitted = Native-LifecycleServer
    Assert-NativeLifecycle ($nativeLifecycleCommitted.counts.transactions -eq $nativeLifecycleBaseline.counts.transactions + 1) 'The synthetic create was not committed before losing its acknowledgement.'
    $nativeLifecycleRenewal = Invoke-ExcelSyncFixtureControl $session POST '/__test__/grant' @{owner_id=1;revoke_previous=$true}
    Native-LifecycleUseGrant ([string]$nativeLifecycleRenewal.credentials_path) '1'
    Native-LifecycleUnpause
    [void](Native-LifecycleSync)
    $nativeLifecycleRenewed = Native-LifecycleServer
    $nativeLifecycleCreated = @($nativeLifecycleRenewed.records | Where-Object { $_.fields.annotation_details -ceq $nativeLifecycleCreateMarker })
    Assert-NativeLifecycle ($nativeLifecycleCreated.Count -eq 1) 'Same-owner renewal duplicated the create.'
    $nativeLifecycleCreatedId = [string]$nativeLifecycleCreated[0].id
    Assert-NativeLifecycle ($nativeLifecycleRenewed.counts.transactions -eq $nativeLifecycleBaseline.counts.transactions + 1) 'Unexpected receipt count after renewal.'
    Assert-NativeLifecycle (@($nativeLifecycleRenewed.operations | Where-Object { $_.operation_id -ceq $nativeLifecycleCreateOperationId }).Count -eq 1) 'Renewal duplicated the original operation ledger entry.'
    Assert-NativeLifecycle ((Native-LifecycleEntries (Native-LifecycleClient).outbox).Count -eq 0) 'Renewed grant did not acknowledge the original create.'
    $nativeLifecycleReport.checks += 'Native lost create ACK -> revoked grant -> same-owner renewal -> original operation, one receipt'

    # Produce an immutable pending update before replacing WinCred outside Connect.
    $nativeLifecycleReport.phase = 'other_owner_rejected'
    $nativeLifecycleEditMarker = 'native-lifecycle-offline-' + [guid]::NewGuid().ToString('N')
    Native-LifecycleUnpause
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StartSync')
    Native-LifecycleFault @{fail_next_sync=1}
    [void](Set-ExcelSyncTestFields $book $session $nativeLifecycleCreatedId @{annotation_details=$nativeLifecycleEditMarker})
    [void](Native-LifecycleSync)
    $nativeLifecycleBeforeOwnerSwap = Native-LifecycleClient
    $nativeLifecycleUpdateEntries = Native-LifecycleEntries $nativeLifecycleBeforeOwnerSwap.outbox
    Assert-NativeLifecycle ($nativeLifecycleUpdateEntries.Count -eq 1 -and $nativeLifecycleUpdateEntries[0].Value.kind -eq 'update') 'Expected one durable update before owner replacement.'
    $nativeLifecycleUpdateId = [string]$nativeLifecycleUpdateEntries[0].Name
    $nativeLifecycleQueueBeforeSwap = Native-LifecycleQueue $nativeLifecycleBeforeOwnerSwap
    $nativeLifecycleOther = Invoke-ExcelSyncFixtureControl $session POST '/__test__/grant' @{owner_id=2;revoke_previous=$false}
    Native-LifecycleUseGrant ([string]$nativeLifecycleOther.credentials_path) '2'
    Native-LifecycleUnpause
    $nativeLifecycleServerBeforeSwap = Native-LifecycleServer
    [void](Native-LifecycleSync)
    $nativeLifecycleWrongOwner = Native-LifecycleClient
    $nativeLifecycleServerAfterSwap = Native-LifecycleServer
    Assert-NativeLifecycle ($nativeLifecycleWrongOwner.paused -eq $true) 'Wrong-owner credential did not pause native synchronization.'
    Assert-NativeLifecycle ([string]$nativeLifecycleWrongOwner.owner_id -ceq '1') 'Wrong-owner snapshot changed the persistent owner binding.'
    Assert-NativeLifecycle ((Native-LifecycleQueue $nativeLifecycleWrongOwner) -ceq $nativeLifecycleQueueBeforeSwap) 'Wrong-owner credential changed an intention, operation or conflict.'
    Assert-NativeLifecycle ($nativeLifecycleServerAfterSwap.counts.operations -eq $nativeLifecycleServerBeforeSwap.counts.operations) 'Wrong-owner credential reached a write operation.'
    $nativeLifecycleTail = @($nativeLifecycleServerAfterSwap.requests | Select-Object -Last 2)
    Assert-NativeLifecycle ($nativeLifecycleTail.Count -eq 2 -and $nativeLifecycleTail[0].method -eq 'GET' -and $nativeLifecycleTail[0].path -eq '/__test__/state' -and $nativeLifecycleTail[1].method -eq 'GET' -and $nativeLifecycleTail[1].path -eq '/api/excel/v1/snapshot') 'Expected only the owner-check GET after the observation boundary, with no protocol POST.'
    Assert-NativeLifecycle ((Native-LifecycleRecord $nativeLifecycleServerAfterSwap $nativeLifecycleCreatedId).fields.annotation_details -ceq $nativeLifecycleCreateMarker) 'Wrong-owner request changed synthetic receipt content.'
    $nativeLifecycleReport.checks += 'Native WinCred replaced by owner 2 -> initial GET only, no POST, original owner and immutable queue preserved'

    # Resume owner 1 while the SAME fixture is offline. Exercise actual document events.
    $nativeLifecycleReport.phase = 'native_save_close_reopen_offline'
    Native-LifecycleFault @{offline=$true;fail_next_sync=0;lose_next_sync_ack=$false}
    Native-LifecycleUseGrant ([string]$nativeLifecycleRenewal.credentials_path) '1'
    Native-LifecycleUnpause
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StartSync')
    [void](Invoke-ExcelSyncTestSync $book $session)
    $nativeLifecycleQueueBeforeClose = Native-LifecycleQueue (Native-LifecycleClient)
    Assert-NativeLifecycle ($nativeLifecycleQueueBeforeClose -ceq $nativeLifecycleQueueBeforeSwap) 'Offline resume changed the pending immutable update.'
    $excel.EnableEvents = $true
    $book.Save()
    Assert-NativeLifecycle ([bool]$book.Saved) 'Native Save/AfterSave left the book dirty. Inspect the Save/Close fix before testing Close.'
    # Requires PrepareClose to capture Saved BEFORE PersistBeforeSave changes protection.
    # No false Saved assignment, events suppression, or automatic dialog response is used.
    $book.Close($false)
    $book = $null
    $nativeLifecycleClosed = $true
    $nativeLifecycleLock = Join-Path ([IO.Path]::GetDirectoryName([string]$session.state_path)) 'client.lock'
    $nativeLifecycleLockHandle = [IO.File]::Open($nativeLifecycleLock, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $nativeLifecycleLockHandle.Dispose()
    Assert-NativeLifecycle ((Native-LifecycleQueue (Native-LifecycleClient)) -ceq $nativeLifecycleQueueBeforeClose) 'Native close lost the durable pending queue.'
    $excel.EnableEvents = $true
    $book = $excel.Workbooks.Open($nativeLifecycleBookPath, 0, $false)
    $nativeLifecycleClosed = $false
    # Do not call StartSync here: Workbook_Open must have restored the client itself.
    $nativeLifecycleReopened = Native-LifecycleClient
    Assert-NativeLifecycle ([string]$book.Worksheets.Item('_sync').Range('B1').Value2 -ceq [string]$session.workbook_id) 'Reopening replaced the workbook ID.'
    Assert-NativeLifecycle ((Native-LifecycleQueue $nativeLifecycleReopened) -ceq $nativeLifecycleQueueBeforeClose) 'Workbook_Open changed the preserved queue.'
    $nativeLifecycleSheet = $book.Worksheets.Item([string]$book.Worksheets.Item('_sync').Range('B3').Value2)
    $nativeLifecycleRow = Get-ExcelSyncTestRow $book $nativeLifecycleCreatedId
    Assert-NativeLifecycle ([string]$nativeLifecycleSheet.Cells.Item($nativeLifecycleRow,37).Value2 -ceq $nativeLifecycleEditMarker) 'Workbook_Open did not restore the unsent edit into the grid.'
    Assert-NativeLifecycle (-not [bool]$nativeLifecycleSheet.Cells.Item($nativeLifecycleRow,37).Locked) 'Workbook_Open did not restore editable native inputs.'
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StopSync')
    Assert-NativeLifecycle ((Native-LifecycleRecord (Native-LifecycleServer) $nativeLifecycleCreatedId).fields.annotation_details -ceq $nativeLifecycleCreateMarker) 'Offline save/reopen unexpectedly wrote the pending edit.'
    $nativeLifecycleReport.checks += 'Real Workbook_BeforeSave/AfterSave/BeforeClose/Open with events enabled -> lock released, same workbook ID, pending edit restored'

    $nativeLifecycleReport.phase = 'reconnect_flush_and_cleanup'
    Native-LifecycleFault @{offline=$false}
    Native-LifecycleUnpause
    [void](Native-LifecycleSync)
    $nativeLifecycleRecovered = Native-LifecycleServer
    Assert-NativeLifecycle ((Native-LifecycleRecord $nativeLifecycleRecovered $nativeLifecycleCreatedId).fields.annotation_details -ceq $nativeLifecycleEditMarker) 'Reconnection did not flush the reopened offline edit.'
    Assert-NativeLifecycle (@($nativeLifecycleRecovered.operations | Where-Object { $_.operation_id -ceq $nativeLifecycleUpdateId }).Count -eq 1) 'Reconnection did not use exactly one original update operation.'
    Assert-NativeLifecycle ((Native-LifecycleEntries (Native-LifecycleClient).intents).Count -eq 0 -and (Native-LifecycleEntries (Native-LifecycleClient).outbox).Count -eq 0) 'Recovered update remained pending.'
    $nativeLifecycleReport.checks += 'Correct-owner reconnect after native reopen -> original queued update applied once and acknowledged'
    # Remove only the exact synthetic receipt this script created after it was verified.
    Add-ExcelSyncConfirmedTestDelete $book $session $nativeLifecycleCreatedId
    [void](Native-LifecycleSync)
    $nativeLifecycleFinal = Native-LifecycleServer
    Assert-NativeLifecycle (@($nativeLifecycleFinal.records | Where-Object { [string]$_.id -ceq $nativeLifecycleCreatedId }).Count -eq 0) 'Synthetic lifecycle receipt cleanup failed.'
    Assert-NativeLifecycle ($nativeLifecycleFinal.counts.transactions -eq $nativeLifecycleBaseline.counts.transactions) 'Lifecycle cleanup affected an unexpected receipt count.'
    foreach ($nativeLifecycleExisting in $nativeLifecycleBaseline.records) {
        $nativeLifecyclePreserved = Native-LifecycleRecord $nativeLifecycleFinal ([string]$nativeLifecycleExisting.id)
        Assert-NativeLifecycle (($nativeLifecyclePreserved.fields | ConvertTo-Json -Depth 30 -Compress) -ceq ($nativeLifecycleExisting.fields | ConvertTo-Json -Depth 30 -Compress)) 'An existing fixture receipt changed during lifecycle testing.'
    }
    Assert-NativeLifecycle ((Native-LifecycleEntries (Native-LifecycleClient).outbox).Count -eq 0 -and (Native-LifecycleEntries (Native-LifecycleClient).blocked).Count -eq 0) 'Lifecycle left pending operations or conflicts.'
    [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StartSync')
    $excel.EnableEvents = $true
    $book.Save()
    $nativeLifecycleReport.checks += 'Only this script synthetic receipt removed; existing fixture receipts preserved'
    $nativeLifecycleReport.phase = 'complete'
    $nativeLifecycleReport.passed = $true
    $nativeLifecycleSuccess = $true
} catch {
    $nativeLifecycleReport.error = $_.Exception.Message
    throw
} finally {
    if ($nativeLifecycleFixtureValidated) {
        try { Native-LifecycleFault @{offline=$false;fail_next_snapshot=0;fail_next_sync=0;lose_next_sync_ack=$false} } catch {}
    }
    if ($nativeLifecycleClosed) {
        try {
            $excel.EnableEvents = $true
            $book = $excel.Workbooks.Open($nativeLifecycleBookPath, 0, $false)
            $nativeLifecycleClosed = $false
        } catch { $nativeLifecycleReport.reopen_recovery_failed = $true }
    }
    if (-not $nativeLifecycleSuccess -and $null -ne $book -and $null -ne $nativeLifecycleRenewal) {
        try {
            Native-LifecycleUseGrant ([string]$nativeLifecycleRenewal.credentials_path) '1'
            $nativeLifecycleFailureState = Native-LifecycleClient
            $nativeLifecycleFailureState.paused = $true
            Native-LifecycleSaveState $nativeLifecycleFailureState
            [void](Invoke-ExcelSyncMacro $book 'ExcelSync.StartSync')
            $nativeLifecycleReport.recovery = 'Correct-owner WinCred restored; queue kept and paused for inspection.'
        } catch { $nativeLifecycleReport.credential_recovery_failed = $true }
    }
    $excel.EnableEvents = $nativeLifecycleOriginalEvents
    $nativeLifecycleReport | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $nativeLifecycleReportPath -Encoding UTF8
}
[pscustomobject]$nativeLifecycleReport
