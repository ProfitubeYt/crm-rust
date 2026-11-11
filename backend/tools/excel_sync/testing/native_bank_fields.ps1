param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
# Caller-owned Excel and an explicit synthetic test copy only. This harness
# never opens/closes Excel, injects local intents, or accesses a production API.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if ($Session -is [string]) { $Session = Get-Content -LiteralPath $Session -Raw -Encoding UTF8 | ConvertFrom-Json }
$bfApp = $Workbook.Application
$bfOriginalEvents = $bfApp.EnableEvents
$bfValidated = $false
$bfReportPath = $null
$bfMapping = [ordered]@{
    transaction_date=17;transaction_type=20;operator_raw=21;application_mapped=23;amount=26;
    balance_after=33;card_last_4=34;is_p2p=35;p2p_confirmed=36;annotation_details=37;
    annotation_person_code=38;annotation_period=39;annotation_article_code=40;currency=41;
    receiver_name=42;receiver_card=43;raw_message=44
}
$bfReport = [ordered]@{native=$true;passed=$false;phase='validate';workbook_id=$Session.workbook_id;
    fields=@($bfMapping.Keys);checks=@();banks=@()}

function B-Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function B-Phase([string]$Phase) {
    $script:bfReport.phase = $Phase
    $script:bfReport.updated_utc = [datetime]::UtcNow.ToString('o')
    if ($script:bfReportPath) { $script:bfReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $script:bfReportPath -Encoding UTF8 }
}
function B-Server { return Invoke-ExcelSyncFixtureControl $Session GET '/__test__/state' }
function B-State { return Read-ExcelSyncTestState $Session }
function B-Fault([bool]$Offline) { [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/fault' @{offline=$Offline}) }
function B-Sync {
    $result = Invoke-ExcelSyncTestSync $Workbook $Session
    $script:bfReport.last_sync_status = [string]$result.status
}
function B-First([string]$Bank) { if ($Bank -ceq 'HUMO Card') { return 4 }; return 5 }
function B-Column([string]$Bank, [int]$Core) {
    if ($Bank -cne 'HUMO Card') { return $Core }
    if ($Core -ge 15 -and $Core -le 25) { return $Core }
    if ($Core -eq 26) { return 32 }
    if ($Core -eq 32) { return 33 }
    if ($Core -ge 33 -and $Core -le 53) { return $Core+7 }
    throw "Unsupported bank/core column: $Bank/$Core"
}
function B-Activate([string]$Bank) {
    $Workbook.Activate()
    $Workbook.Worksheets.Item($Bank).Activate()
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.ResumeView')
}
function B-Row([string]$Bank, [string]$Id) {
    $sheet = $Workbook.Worksheets.Item($Bank)
    $column = B-Column $Bank 51
    $last = [int]$sheet.Cells.Item($sheet.Rows.Count, (B-Column $Bank 52)).End(-4162).Row
    for ($row=(B-First $Bank); $row -le $last; $row++) {
        if ([string]$sheet.Cells.Item($row,$column).Value2 -ceq $Id) { return $row }
    }
    throw "Synthetic receipt $Id is missing from $Bank."
}
function B-Record([string]$Id) {
    $found = @((B-Server).records | Where-Object {[string]$_.id -ceq $Id})
    B-Assert ($found.Count -eq 1) "Expected one synthetic receipt $Id."
    return $found[0]
}
function B-EmptyQueue {
    $state = B-State
    foreach ($name in @('intents','outbox','blocked')) {
        B-Assert (@($state.$name.PSObject.Properties).Count -eq 0) "Expected an empty $name queue."
    }
}
function B-LockDiagnostic([string]$Bank, [int]$Row, [int]$Column) {
    $sheet = $Workbook.Worksheets.Item($Bank)
    $state = B-State
    return [ordered]@{
        bank=$Bank;row=$Row;column=$Column;events=[bool]$bfApp.EnableEvents;
        active_workbook=[string]$bfApp.ActiveWorkbook.Name;active_sheet=[string]$bfApp.ActiveSheet.Name;
        can_edit=[bool](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.ViewCanEdit');
        can_render_edit=[bool](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.ViewCanRenderEdit');
        paused=$state.paused;snapshot_received=$state.snapshot_received;
        protect_contents=[bool]$sheet.ProtectContents;protection_mode=[bool]$sheet.ProtectionMode;
        status=[string]$sheet.Cells.Item(1,(B-Column $Bank 50)).Value2
    }
}
function B-Set([string]$Bank, [int]$Row, [System.Collections.IDictionary]$Fields) {
    B-Assert ($Row -ge (B-First $Bank)) 'Only bank body/input rows may be edited.'
    $sheet = $Workbook.Worksheets.Item($Bank)
    foreach ($field in $Fields.Keys) {
        B-Assert ($bfMapping.Contains($field)) "Unknown editable bank field: $field"
        $column = B-Column $Bank ([int]$bfMapping[$field])
        $cell = $sheet.Cells.Item($Row,$column)
        if ([bool]$cell.Locked) {
            $diagnostic = B-LockDiagnostic $Bank $Row $column
            $script:bfReport.lock_diagnostic = $diagnostic
            throw "Bank input is locked: $Bank R${Row}C${column} ($field). Diagnostic: $($diagnostic | ConvertTo-Json -Compress)"
        }
        $value = $Fields[$field]
        if ($null -eq $value) {
            [void]$cell.ClearContents()
        } else {
            switch ($field) {
                {$_ -in @('transaction_date','annotation_period')} { $value=([datetime]$value).ToOADate() }
                {$_ -in @('amount','balance_after')} { $value=[double]::Parse([string]$value,[Globalization.CultureInfo]::InvariantCulture) }
                {$_ -in @('is_p2p','p2p_confirmed')} { $value=[double][bool]$value }
                default { $value=[string]$value }
            }
            # Explicit VARIANT avoids Windows PowerShell 5 caching a BSTR setter.
            $put = [object[]]::new(1); $put[0] = $value
            [void]$cell.GetType().InvokeMember('Value2',[Reflection.BindingFlags]::SetProperty,
                $null,$cell,$put,[Globalization.CultureInfo]::GetCultureInfo('en-US'))
        }
    }
}
function B-Stage([string]$Bank, [string]$Id, [System.Collections.IDictionary]$Fields) {
    B-Activate $Bank
    B-Fault $true
    try { B-Set $Bank (B-Row $Bank $Id) $Fields }
    finally { B-Fault $false }
    B-Sync
    B-EmptyQueue
}
function B-Canonical([string]$Id, [System.Collections.IDictionary]$Expected) {
    $actual = (B-Record $Id).fields
    foreach ($field in $Expected.Keys) {
        $property = $actual.PSObject.Properties[$field]
        B-Assert ($null -ne $property) "Missing canonical field $field."
        $value = $property.Value; $want = $Expected[$field]
        if ($null -eq $want) {
            B-Assert ($null -eq $value) "Canonical $field did not clear to null."
        } elseif ($want -is [bool]) {
            B-Assert (($value -is [bool]) -and ($value -eq $want)) "Canonical Boolean $field changed value/type."
        } else {
            $text = [string]$value
            if (($value -is [datetime] -or $value -is [datetimeoffset]) -and $field -in @('transaction_date','annotation_period')) {
                $format = if ($field -eq 'annotation_period') { 'yyyy-MM-dd' } else { "yyyy-MM-dd'T'HH:mm:ss.FFFFFFF" }
                $text = $value.ToString($format,[Globalization.CultureInfo]::InvariantCulture)
            }
            B-Assert ($text -ceq [string]$want) "Canonical field $field differs for receipt $Id."
        }
    }
}
function B-Grid([string]$Bank, [string]$Id, [System.Collections.IDictionary]$Expected) {
    $sheet = $Workbook.Worksheets.Item($Bank); $row = B-Row $Bank $Id
    $prototypeName = if ($Bank -ceq 'HUMO Card') { '_styleHumo' } else { '_styleCard' }
    $prototype = $Workbook.Worksheets.Item($prototypeName)
    foreach ($field in $Expected.Keys) {
        if ($field -eq 'excel_bank') { continue }
        $cell = $sheet.Cells.Item($row,(B-Column $Bank ([int]$bfMapping[$field])))
        $value = $cell.Value2; $want = $Expected[$field]
        B-Assert (-not [bool]$cell.HasFormula) "$Bank $field became a formula."
        if ($field -in @('amount','balance_after')) {
            # Bank reference layouts intentionally display whole units. Preserve
            # that exact format independently of the cents stored in Value2.
            $originalFormat = [string]$prototype.Cells.Item(5,(B-Column $Bank ([int]$bfMapping[$field]))).NumberFormat
            B-Assert ([string]$cell.NumberFormat -ceq $originalFormat) "$Bank $field changed the original bank number format."
        }
        if ($null -eq $want -or ($want -is [string] -and $want.Length -eq 0)) {
            B-Assert ([string]::IsNullOrEmpty([string]$value)) "$Bank $field is not blank after clear."
        } elseif ($field -in @('transaction_date','annotation_period')) {
            $date = [datetime]::Parse([string]$want,[Globalization.CultureInfo]::InvariantCulture)
            B-Assert ([Math]::Abs([double]$value-$date.ToOADate()) -lt 0.0000001) "$Bank $field changed local date/time."
        } elseif ($field -in @('amount','balance_after')) {
            $number = [double]::Parse([string]$want,[Globalization.CultureInfo]::InvariantCulture)
            if ($field -eq 'amount') { $number=[Math]::Abs($number) }
            B-Assert ([Math]::Abs([double]$value-$number) -lt 0.0000001) "$Bank $field lost cents/sign."
        } elseif ($field -in @('is_p2p','p2p_confirmed')) {
            B-Assert (-not [string]::IsNullOrEmpty([string]$value) -and [double]$value -eq [double][bool]$want) "$Bank $field is not the expected numeric 0/1."
        } elseif ($field -eq 'annotation_article_code') {
            B-Assert ([string]$value -ceq ([string]$want+'='+$bfArticleLabels[[string]$want])) "$Bank article code=label rendering failed."
        } elseif ($field -eq 'transaction_type') {
            B-Assert (-not [string]::IsNullOrWhiteSpace([string]$value) -and [string]$value -cne [string]$want) "$Bank operation was not rendered as a decorated label."
        } else {
            B-Assert ([string]$value -ceq [string]$want) "$Bank $field changed Unicode/text/leading zeros."
        }
    }
    B-Assert ([double]$sheet.Rows.Item($row).RowHeight -eq 12) "$Bank receipt row lost its 12-point height."
}

try {
    B-Assert ([string]$Session.test_marker -ceq 'TBSparcer isolated native Excel test') 'Expected a synthetic native session.'
    B-Assert ([string]$Session.api_url -ceq 'http://127.0.0.1:8767') 'Only the isolated loopback fixture is allowed.'
    $testRoot = [IO.Path]::GetFullPath([string]$Session.test_root).TrimEnd('\','/')
    $bookPath = [IO.Path]::GetFullPath([string]$Workbook.FullName)
    B-Assert ($bookPath.StartsWith($testRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) 'Workbook is outside the synthetic test root.'
    B-Assert ($bookPath -ceq [IO.Path]::GetFullPath([string]$Session.workbook_path) -and [IO.Path]::GetExtension($bookPath) -ceq '.xlsm' -and -not [bool]$Workbook.ReadOnly) 'Use the writable synthetic .xlsm session copy.'
    $config = $Workbook.Worksheets.Item('_sync')
    B-Assert ([string]$config.Range('B1').Value2 -ceq [string]$Session.workbook_id) 'Workbook/session identity mismatch.'
    B-Assert ([string]$config.Range('B5').Value2 -ceq [string]$Session.api_url) 'Workbook contains a different server origin.'
    B-Assert ([int]$bfApp.CutCopyMode -eq 0) 'Finish the current Excel cut/copy before this test.'
    B-Assert ($bfMapping.Count -eq 17) 'Expected all 17 visible canonical editable fields.'
    $fixture = B-Server
    B-Assert ($fixture.synthetic -and [string]$fixture.instance_id -ceq [string]$Session.instance_id) 'Synthetic server identity mismatch.'
    B-Assert (-not $fixture.faults.offline -and @($fixture.records).Count -ge 3) 'Use an online synthetic fixture with receipts 2 and 3.'
    $bfValidated = $true
    $bfReportPath = Join-Path $testRoot 'native-bank-fields-verification.json'
    B-EmptyQueue
    $bfApp.EnableEvents = $true
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StartSync')
    B-Activate 'CardXabar'
    B-Sync
    B-EmptyQueue
    $untouched = @{}
    foreach ($record in (B-Server).records) {
        if ([string]$record.id -notin @('2','3')) { $untouched[[string]$record.id]=$record | ConvertTo-Json -Depth 50 -Compress }
    }
    $refs = $Workbook.Worksheets.Item('_refs')
    $bfPerson = [string]$refs.Range('A2').Value2
    $bfArticle = [string]$refs.Range('C2').Value2
    $bfArticleLabels = @{}
    $lastReference = [int]$refs.Cells.Item($refs.Rows.Count,3).End(-4162).Row
    for ($row=2; $row -le $lastReference; $row++) { $bfArticleLabels[[string]$refs.Cells.Item($row,3).Value2]=[string]$refs.Cells.Item($row,4).Value2 }
    B-Assert ($bfPerson.Length -gt 0 -and $bfArticle.Length -gt 0 -and $bfArticleLabels[$bfArticle].Length -gt 0) 'Fixture person/article reference lists are empty.'
    # Keep the source ASCII for Windows PowerShell 5 while exercising Unicode.
    $unicode = '{"operator":"\u041a\u0430\u0444\u0435 \u0401\u0436\u0438\u043a","app":"\u0418\u043b\u043e\u0432\u0430 / native","details":"=\u042e\u043d\u0438\u043a\u043e\u0434 \u2014 bank fixture","receiver":"\u041e\u043b\u0451\u043d\u0430 \u04b2\u0430\u0441\u0430\u043d\u043e\u0432\u0430","raw":"\ud83d\udd34 Native decorated operation\n\u0412\u0442\u043e\u0440\u0430\u044f \u0441\u0442\u0440\u043e\u043a\u0430 \ud83d\udcb3"}' | ConvertFrom-Json
    $cases = @(@{bank='CardXabar';id='2';kind='DEBIT';amount='-9876.54';balance='-12.30'},
        @{bank='HUMO Card';id='3';kind='CREDIT';amount='7654.01';balance='0.01'})
    B-Phase 'assign_synthetic_bank_receipts'
    foreach ($case in $cases) {
        [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/server-edit' @{id=[int]$case.id;fields=@{excel_bank=$case.bank}})
    }
    foreach ($case in $cases) { B-Activate $case.bank; B-Sync; [void](B-Row $case.bank $case.id) }

    foreach ($case in $cases) {
        $bank=[string]$case.bank; $id=[string]$case.id
        B-Phase ($bank+'_all_17_native_fields')
        $expected = [ordered]@{
            transaction_date='2026-08-31T23:59:58';transaction_type=$case.kind;operator_raw=($unicode.operator+' '+$bank);
            application_mapped=$unicode.app;amount=$case.amount;balance_after=$case.balance;card_last_4='0007';
            is_p2p=$true;p2p_confirmed=$false;annotation_details=$unicode.details;annotation_person_code=$bfPerson;
            annotation_period='2026-07-01';annotation_article_code=$bfArticle;currency='USD';
            receiver_name=$unicode.receiver;receiver_card='0000123456789012';raw_message=$unicode.raw
        }
        $inputFields = [ordered]@{}
        foreach ($field in $expected.Keys) { $inputFields[$field]=$expected[$field] }
        $inputFields.annotation_article_code=$bfArticle+'='+$bfArticleLabels[$bfArticle]
        B-Stage $bank $id $inputFields
        B-Canonical $id $expected
        B-Grid $bank $id $expected
        B-Canonical $id @{excel_bank=$bank}
        $bfReport.checks += "${bank}: all 17 native SheetChange edits round-trip; code=label becomes code; Unicode, literal equals, leading zeros, exact cents and local dates"

        B-Phase ($bank+'_decorated_operation_noop')
        $before = B-Server
        $beforeRecord = (B-Record $id) | ConvertTo-Json -Depth 50 -Compress
        $row = B-Row $bank $id
        $decorated = [string]$Workbook.Worksheets.Item($bank).Cells.Item($row,20).Value2
        B-Assert ($decorated -ceq ($unicode.raw -split "`n")[0]) 'Expected the original decorated operation headline.'
        B-Fault $true
        try { B-Set $bank $row @{transaction_type=$decorated}; B-EmptyQueue }
        finally { B-Fault $false }
        B-Sync
        B-EmptyQueue
        B-Assert ([int](B-Server).counts.operations -eq [int]$before.counts.operations) 'Copying the unchanged decorated operation generated an API operation.'
        B-Assert (((B-Record $id) | ConvertTo-Json -Depth 50 -Compress) -ceq $beforeRecord) 'Copying the decorated operation changed the server receipt/version.'
        $bfReport.checks += "${bank}: copying unchanged decorated T creates no intent/outbox/API operation and preserves the server version"

        B-Phase ($bank+'_nullable_independent_p2p')
        foreach ($flags in @(@{p2p_confirmed=$true},@{is_p2p=$false},@{p2p_confirmed=$null})) {
            B-Stage $bank $id $flags
            foreach ($field in $flags.Keys) { $expected[$field]=$flags[$field] }
            B-Canonical $id $expected
            B-Grid $bank $id $expected
        }
        $clears = @{application_mapped=$null;balance_after=$null;receiver_name=$null;receiver_card=$null;
            annotation_details=$null;annotation_person_code=$null;annotation_period=$null;annotation_article_code=$null;raw_message=''}
        B-Stage $bank $id $clears
        foreach ($field in $clears.Keys) { $expected[$field]=$clears[$field] }
        B-Canonical $id $expected
        B-Grid $bank $id $expected
        $bfReport.checks += "${bank}: p2p_confirmed false/true/null is independent of is_p2p; optional fields clear to null and raw text to empty"

        B-Phase ($bank+'_native_draft_implicit_bank')
        B-Activate $bank
        $sheet = $Workbook.Worksheets.Item($bank)
        $draftRow = [Math]::Max((B-First $bank),[int]$sheet.Cells.Item($sheet.Rows.Count,(B-Column $bank 52)).End(-4162).Row+1)
        $marker = 'native-bank-draft-'+[guid]::NewGuid().ToString('N')
        $before = B-Server
        B-Fault $true
        try {
            B-Set $bank $draftRow @{operator_raw=$marker}
            $draftUuid=[string]$sheet.Cells.Item($draftRow,(B-Column $bank 52)).Value2
            $parsed=[guid]::Empty
            B-Assert ([guid]::TryParse($draftUuid,[ref]$parsed)) 'The native bank draft did not receive a UUID.'
            B-Assert ([string]::IsNullOrEmpty([string]$sheet.Cells.Item($draftRow,(B-Column $bank 51)).Value2)) 'The incomplete draft already has a server ID.'
            $intent=(B-State).intents.PSObject.Properties['u:'+$draftUuid]
            B-Assert ($null -ne $intent -and [string]$intent.Value.kind -ceq 'draft') 'A native bank edit did not create a durable incomplete draft.'
            B-Assert ([string]$intent.Value.fields.excel_bank -ceq $bank) 'The new draft did not inherit its bank sheet.'
            B-Assert ($null -eq (B-State).records.PSObject.Properties['u:'+$draftUuid].Value.fields.p2p_confirmed) 'A fresh draft confirmation must default to null.'
            $draftExpected=[ordered]@{}
            foreach ($field in $inputFields.Keys) { $draftExpected[$field]=$inputFields[$field] }
            $draftExpected.operator_raw=$marker
            $draftExpected.p2p_confirmed=$true
            B-Set $bank $draftRow $draftExpected
            B-Assert ([string]$sheet.Cells.Item($draftRow,(B-Column $bank 52)).Value2 -ceq $draftUuid) 'Completing the draft replaced its local UUID.'
            B-Assert ([int](B-Server).counts.transactions -eq [int]$before.counts.transactions) 'An offline draft was sent prematurely.'
        } finally { B-Fault $false }
        B-Sync
        B-EmptyQueue
        $after=B-Server
        $created=@($after.records | Where-Object {[string]$_.fields.operator_raw -ceq $marker})
        B-Assert ($created.Count -eq 1) 'The native bank draft must create exactly one receipt.'
        B-Assert ([int]$after.counts.transactions -eq [int]$before.counts.transactions+1) 'Unexpected transaction count after draft acknowledgement.'
        B-Assert ([int]$after.counts.operations -eq [int]$before.counts.operations+1) 'A native draft must commit exactly one create operation.'
        $draftExpected.annotation_article_code=$bfArticle
        $draftExpected.excel_bank=$bank
        B-Canonical ([string]$created[0].id) $draftExpected
        B-Grid $bank ([string]$created[0].id) $draftExpected
        B-Assert ((B-Row $bank ([string]$created[0].id)) -eq $draftRow) 'The acknowledged draft changed its bank row.'
        $bfReport.banks += @{bank=$bank;first_body_row=(B-First $bank);edited_receipt_id=$id;
            draft_id=[string]$created[0].id;draft_local_uuid=$draftUuid;draft_server_uuid=[string]$created[0].uuid}
        $bfReport.checks += "${bank}: native partial draft inherits excel_bank, defaults confirmation to null, and all 17 fields create exactly one receipt in place"
    }
    foreach ($id in $untouched.Keys) {
        B-Assert (((B-Record $id) | ConvertTo-Json -Depth 50 -Compress) -ceq $untouched[$id]) "Unrelated synthetic receipt $id changed."
    }
    $bfReport.passed=$true
    B-Phase 'complete'
} catch {
    $bfReport.error=$_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'
    $bfReport.error_line=$_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    if ($bfValidated) {
        try { [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync') } catch { $bfReport.stop_error=$_.Exception.Message }
        try { B-Fault $false } catch { $bfReport.fixture_cleanup_error=$_.Exception.Message }
    }
    $bfApp.EnableEvents=$bfOriginalEvents
    if ($bfReportPath) { $bfReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $bfReportPath -Encoding UTF8 }
}
[pscustomobject]$bfReport
