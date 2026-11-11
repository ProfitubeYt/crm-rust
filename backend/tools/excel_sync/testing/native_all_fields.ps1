param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
# The caller owns Excel. Only synthetic receipt 2 is changed; receipt 1 is retained.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if ($Session -is [string]) { $Session = Get-Content -LiteralPath $Session -Raw -Encoding UTF8 | ConvertFrom-Json }
if ([string]$Workbook.FullName -ne [string]$Session.workbook_path) { throw 'Workbook/session mismatch.' }
if ([string]$Workbook.Worksheets.Item('_sync').Range('B1').Value2 -ne [string]$Session.workbook_id) { throw 'Workbook ID mismatch.' }

$mapping = [ordered]@{transaction_date=17;transaction_type=20;operator_raw=21;application_mapped=23;amount=26;
    balance_after=33;card_last_4=34;is_p2p=35;annotation_details=37;annotation_person_code=38;
    annotation_period=39;annotation_article_code=40;currency=41;receiver_name=42;receiver_card=43;raw_message=44}
$sheet = $Workbook.Worksheets.Item([string]$Workbook.Worksheets.Item('_sync').Range('B3').Value2)
$reportPath = Join-Path $Session.test_root 'native-all-fields-verification.json'
$verification = [ordered]@{native=$true;workbook_id=$Session.workbook_id;passed=$false;checks=@();fields=@($mapping.Keys)}

function Assert-That([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Fixture-State { return Invoke-ExcelSyncFixtureControl $Session GET '/__test__/state' }
function Local-State { return Read-ExcelSyncTestState $Session }
function Sync-Client { return Invoke-ExcelSyncTestSync $Workbook $Session }
function Offline([bool]$Value) { [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/fault' @{offline=$Value}) }
function Server-Record([string]$Id) {
    $found = @((Fixture-State).records | Where-Object {[string]$_.id -ceq $Id})
    Assert-That ($found.Count -eq 1) "Expected one synthetic receipt $Id."
    return $found[0]
}
function Receipt-One-State {
    $state = Local-State
    $record = @($state.records.PSObject.Properties | Where-Object {[string]$_.Value.id -ceq '1'})[0]
    $key = [string]$record.Name
    $intent = $state.intents.PSObject.Properties[$key]
    $blocked = $state.blocked.PSObject.Properties[$key]
    $operations = @($state.outbox.PSObject.Properties | Where-Object {[string]$_.Value.id -ceq '1'} | ForEach-Object {$_.Value})
    return ([ordered]@{record=$record.Value;intent=$intent.Value;blocked=$blocked.Value;outbox=$operations} | ConvertTo-Json -Depth 100 -Compress)
}
function Assert-Canonical([System.Collections.IDictionary]$Expected) {
    $actual = (Server-Record '2').fields
    foreach ($field in $Expected.Keys) {
        $value = $actual.PSObject.Properties[$field].Value
        if ($null -eq $Expected[$field]) {
            Assert-That ($null -eq $value) "Server field $field was not cleared to null."
        } elseif ($Expected[$field] -is [bool]) {
            Assert-That (($value -is [bool]) -and ($value -eq $Expected[$field])) "Server Boolean $field changed type/value."
        } else {
            $actualText = [string]$value
            # Some PowerShell versions eagerly deserialize ISO JSON dates.
            # Preserve their local calendar/time; never shift to UTC here.
            if (($value -is [datetime] -or $value -is [datetimeoffset]) -and $field -in @('transaction_date','annotation_period')) {
                $format = if ($field -eq 'annotation_period') { 'yyyy-MM-dd' } else { "yyyy-MM-dd'T'HH:mm:ss.FFFFFFF" }
                $actualText = $value.ToString($format, [Globalization.CultureInfo]::InvariantCulture)
            }
            Assert-That ($actualText -ceq [string]$Expected[$field]) "Server field $field differs from the exact expected text."
        }
    }
}
function Assert-Grid([System.Collections.IDictionary]$Expected) {
    $row = Get-ExcelSyncTestRow $Workbook '2'
    foreach ($field in $Expected.Keys) {
        $cell = $sheet.Cells.Item($row, [int]$mapping[$field]); $value = $cell.Value2
        $want = $Expected[$field]
        if ($null -eq $want -or ($want -is [string] -and $want.Length -eq 0)) {
            Assert-That ([string]::IsNullOrEmpty([string]$value)) "Grid field $field is not empty after clear."
        } elseif ($field -in @('transaction_date','annotation_period')) {
            $date = [datetime]::Parse([string]$want, [Globalization.CultureInfo]::InvariantCulture)
            Assert-That ([Math]::Abs([double]$value - $date.ToOADate()) -lt 0.0000001) "Grid date $field changed local calendar/time."
        } elseif ($field -in @('amount','balance_after')) {
            $number = [double]::Parse([string]$want, [Globalization.CultureInfo]::InvariantCulture)
            if ($field -eq 'amount') { $number = [Math]::Abs($number) }
            Assert-That ([Math]::Abs([double]$value - $number) -lt 0.0000001) "Grid decimal $field lost cents/sign."
            Assert-That ([string]$cell.NumberFormat -match '0[.,]00') "Grid decimal $field does not display cents."
        } elseif ($field -eq 'is_p2p') {
            Assert-That ([double]$value -eq [double][bool]$want) 'Grid P2P is not numeric 0/1.'
        } else {
            Assert-That ([string]$value -ceq [string]$want) "Grid text $field changed Unicode/leading zeros."
            Assert-That (-not [bool]$cell.HasFormula) "Grid text $field became a formula."
        }
    }
    Assert-That ([double]$sheet.Rows.Item($row).RowHeight -eq 12) 'Edited receipt height changed from 12 points.'
    $compactTextColumns = @(19,21,23,37,38,40,42,43,44,45,46,47,48,50)
    for ($col=15; $col -le 53; $col++) {
        $cell = $sheet.Cells.Item($row,$col)
        $clipText = $compactTextColumns -contains $col
        Assert-That ($cell.WrapText -eq $clipText) "Column $col lost its text clipping policy after editing."
        if ($clipText) {
            Assert-That ($cell.VerticalAlignment -eq -4160 -and $cell.ShrinkToFit -eq $false) "Column $col must show its first line without shrinking text."
        }
    }
}
function Stage-Fields([hashtable]$Fields) {
    Offline $true
    try { [void](Set-ExcelSyncTestFields $Workbook $Session '2' $Fields) }
    finally { Offline $false }
    [void](Sync-Client)
}
function Grid-Identity {
    $items = [ordered]@{}
    $last = $sheet.Cells.Item($sheet.Rows.Count,52).End(-4162).Row
    for ($row=5; $row -le $last; $row++) {
        $id = [string]$sheet.Cells.Item($row,51).Value2
        if ($id.Length -eq 0) { continue }
        $values = [ordered]@{id=$id;uuid=[string]$sheet.Cells.Item($row,52).Value2;version=[string]$sheet.Cells.Item($row,53).Value2}
        foreach ($field in $mapping.Keys) { $values[$field]=$sheet.Cells.Item($row,[int]$mapping[$field]).Value2 }
        $items[$id] = $values | ConvertTo-Json -Depth 10 -Compress
    }
    return $items
}

try {
    $fixture = Fixture-State
    Assert-That ($fixture.synthetic -and $fixture.instance_id -eq $Session.instance_id) 'Unexpected fixture instance.'
    Assert-That (-not $fixture.faults.offline) 'Fixture must be online before this independent test.'
    $beforeServerOne = (Server-Record '1') | ConvertTo-Json -Depth 50 -Compress
    $beforeLocalOne = Receipt-One-State
    $refs = $Workbook.Worksheets.Item('_refs')
    $person = [string]$refs.Range('A2').Value2; $article = [string]$refs.Range('C2').Value2
    $secondPerson = [string]$refs.Range('A3').Value2; $secondArticle = [string]$refs.Range('C3').Value2
    Assert-That ($person.Length -gt 0 -and $article.Length -gt 0) 'The live fixture reference lists are empty.'
    if ($secondPerson.Length -eq 0) { $secondPerson=$person }
    if ($secondArticle.Length -eq 0) { $secondArticle=$article }
    $verification.references = @{person=$person;article=$article;second_person=$secondPerson;second_article=$secondArticle}
    # ASCII source remains readable by Windows PowerShell 5; JSON supplies Unicode.
    $unicode = '{"operator":"\u041a\u0430\u0444\u0435 \u0401\u0436\u0438\u043a \u2014 \u0422\u043e\u0448\u043a\u0435\u043d\u0442","app":"\u0418\u043b\u043e\u0432\u0430 / test","details":"=\u042e\u043d\u0438\u043a\u043e\u0434 \u2014 \u043f\u0440\u0438\u043c\u0435\u0447\u0430\u043d\u0438\u0435","receiver":"\u041e\u043b\u0451\u043d\u0430 \u04b2\u0430\u0441\u0430\u043d\u043e\u0432\u0430","raw":"=\u0418\u0441\u0445\u043e\u0434\u043d\u044b\u0439 \u0442\u0435\u043a\u0441\u0442\n\u0412\u0442\u043e\u0440\u0430\u044f \u0441\u0442\u0440\u043e\u043a\u0430 \ud83d\udcb3"}' | ConvertFrom-Json
    $clientFields = @{
        transaction_date='2026-08-31T23:59:58';transaction_type='DEBIT';operator_raw=$unicode.operator;
        application_mapped=$unicode.app;amount='-9876.54';balance_after='-12.30';card_last_4='0007';is_p2p=$true;
        annotation_details=$unicode.details;annotation_person_code=$person;annotation_period='2026-07-01';
        annotation_article_code=$article;currency='USD';receiver_name=$unicode.receiver;receiver_card='0000123456789012';raw_message=$unicode.raw
    }
    Assert-That ($clientFields.Count -eq 16) 'The test must cover every canonical editable field.'
    Stage-Fields $clientFields
    Assert-Canonical $clientFields; Assert-Grid $clientFields
    $verification.checks += 'all 16 fields: WorksheetChange -> API/database -> grid; Unicode, emoji, literal equals, leading zeros, exact cents and local dates'

    $clears = @{application_mapped=$null;balance_after=$null;receiver_name=$null;receiver_card=$null;
        annotation_details=$null;annotation_person_code=$null;annotation_period=$null;annotation_article_code=$null;
        raw_message='';is_p2p=$false}
    Stage-Fields $clears
    Assert-Canonical $clears; Assert-Grid $clears
    $verification.checks += 'all 8 nullable editable fields clear to null; raw_message clears to empty string; P2P clears to false'

    $serverFields = @{
        transaction_date='2026-07-30T10:11:12';transaction_type='CREDIT';operator_raw=($unicode.operator + ' server');
        application_mapped=($unicode.app + ' server');amount='7654.01';balance_after='0.01';card_last_4='0019';is_p2p=$true;
        annotation_details=($unicode.details + ' server');annotation_person_code=$secondPerson;annotation_period='2026-12-01';
        annotation_article_code=$secondArticle;currency='UZS';receiver_name=($unicode.receiver + ' server');receiver_card='0000999911112222';raw_message=($unicode.raw + ' server')
    }
    [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/server-edit' @{id=2;fields=$serverFields})
    [void](Sync-Client)
    Assert-Canonical $serverFields; Assert-Grid $serverFields
    $verification.checks += 'all 16 fields: independent server change -> snapshot -> Excel, without overwriting receipt 1 conflict'

    # Formula rejection is tested separately through normal WorksheetChange UI.
    # Calling a private raising procedure through Application.Run can open VBE's
    # unhandled-error dialog and reset VBA state; this script never does that.
    $verification.formula_validation = @{
        included_in_this_run=$false;
        method='Separate Workbook_SheetChange UI verification owned by the caller'
    }

    $identityBefore=Grid-Identity
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.SortByDate')
    $identityAfter=Grid-Identity
    Assert-That ($identityBefore.Count -eq $identityAfter.Count) 'Sort changed the number of visible receipts.'
    foreach ($id in $identityBefore.Keys) { Assert-That ($identityBefore[$id] -ceq $identityAfter[$id]) "Sort detached fields/id/uuid/version for receipt $id." }
    Assert-That ((Get-ExcelSyncTestRow $Workbook '2') -lt (Get-ExcelSyncTestRow $Workbook '1')) 'Date sort did not move receipt 2 ahead of receipt 1.'
    $verification.checks += 'safe date sort moves entire rows and preserves immutable id/uuid/version with all fields'
    Assert-That (((Server-Record '1') | ConvertTo-Json -Depth 50 -Compress) -ceq $beforeServerOne) 'Receipt 1 server data changed during this test.'
    Assert-That ((Receipt-One-State) -ceq $beforeLocalOne) 'Receipt 1 baseline/intents/outbox/conflict changed during this test.'
    Assert-Canonical $serverFields; Assert-Grid $serverFields
    $verification.receipt_1_row=Get-ExcelSyncTestRow $Workbook '1'
    $verification.receipt_2_row=Get-ExcelSyncTestRow $Workbook '2'
    $verification.passed=$true
} catch {
    $verification.error=$_.Exception.Message
    throw
} finally {
    try { Offline $false } catch { }
    $verification | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}
$verification | ConvertTo-Json -Depth 30
