param([Parameter(Mandatory)]$Workbook, [Parameter(Mandatory)]$Session)
# Caller-owned Excel and an explicit synthetic all-transactions view only. This harness
# never opens/closes Excel, injects local intents, or accesses a production API.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'native_fixture.psm1') -Force
if ($Session -is [string]) { $Session = Get-Content -LiteralPath $Session -Raw -Encoding UTF8 | ConvertFrom-Json }
$avApp = $Workbook.Application
$avOriginalEvents = $avApp.EnableEvents
$avValidated = $false
$avReportPath = $null
$avMapping = [ordered]@{
    transaction_date=17;transaction_type=20;operator_raw=21;application_mapped=23;amount=26;
    balance_after=33;card_last_4=34;is_p2p=35;p2p_confirmed=36;annotation_details=37;
    annotation_person_code=38;annotation_period=39;annotation_article_code=40;currency=41;
    receiver_name=42;receiver_card=43;raw_message=44
}
$avReport = [ordered]@{native=$true;passed=$false;phase='validate';workbook_id=$Session.workbook_id;
    fields=@($avMapping.Keys);checks=@();banks=@()}

function A-Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function A-Phase([string]$Phase) {
    $script:avReport.phase = $Phase
    $script:avReport.updated_utc = [datetime]::UtcNow.ToString('o')
    if ($script:avReportPath) { $script:avReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $script:avReportPath -Encoding UTF8 }
}
function A-Server { return Invoke-ExcelSyncFixtureControl $Session GET '/__test__/state' }
function A-State { return Read-ExcelSyncTestState $Session }
function A-Fault([bool]$Offline) { [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/fault' @{offline=$Offline}) }
function A-Sync {
    $result = Invoke-ExcelSyncTestSync $Workbook $Session
    $script:avReport.last_sync_status = [string]$result.status
}
function A-First([string]$Bank) { if ($Bank -ceq 'HUMO Card') { return 4 }; return 5 }
function A-Column([string]$Bank, [int]$Core) {
    if ($Bank -cne 'HUMO Card') { return $Core }
    if ($Core -ge 15 -and $Core -le 25) { return $Core }
    if ($Core -eq 26) { return 32 }
    if ($Core -eq 32) { return 33 }
    if ($Core -ge 33 -and $Core -le 53) { return $Core+7 }
    throw "Unsupported bank/core column: $Bank/$Core"
}
function A-Activate([string]$Bank) {
    $Workbook.Activate()
    $Workbook.Worksheets.Item($Bank).Activate()
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.ResumeView')
}
function A-Row([string]$Bank, [string]$Id) {
    $sheet = $Workbook.Worksheets.Item($Bank)
    $column = A-Column $Bank 51
    $last = [int]$sheet.Cells.Item($sheet.Rows.Count, (A-Column $Bank 52)).End(-4162).Row
    for ($row=(A-First $Bank); $row -le $last; $row++) {
        if ([string]$sheet.Cells.Item($row,$column).Value2 -ceq $Id) { return $row }
    }
    throw "Synthetic receipt $Id is missing from $Bank."
}
function A-Record([string]$Id) {
    $found = @((A-Server).records | Where-Object {[string]$_.id -ceq $Id})
    A-Assert ($found.Count -eq 1) "Expected one synthetic receipt $Id."
    return $found[0]
}
function A-EmptyQueue {
    $state = A-State
    foreach ($name in @('intents','outbox','blocked')) {
        A-Assert (@($state.$name.PSObject.Properties).Count -eq 0) "Expected an empty $name queue."
    }
}
function A-LockDiagnostic([string]$Bank, [int]$Row, [int]$Column) {
    $sheet = $Workbook.Worksheets.Item($Bank)
    $state = A-State
    return [ordered]@{
        bank=$Bank;row=$Row;column=$Column;events=[bool]$avApp.EnableEvents;
        active_workbook=[string]$avApp.ActiveWorkbook.Name;active_sheet=[string]$avApp.ActiveSheet.Name;
        can_edit=[bool](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.ViewCanEdit');
        can_render_edit=[bool](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.ViewCanRenderEdit');
        paused=$state.paused;snapshot_received=$state.snapshot_received;
        protect_contents=[bool]$sheet.ProtectContents;protection_mode=[bool]$sheet.ProtectionMode;
        status=[string]$sheet.Cells.Item(1,(A-Column $Bank 50)).Value2
    }
}
function A-Set([string]$Bank, [int]$Row, [System.Collections.IDictionary]$Fields) {
    A-Assert ($Row -ge (A-First $Bank)) 'Only bank body/input rows may be edited.'
    $sheet = $Workbook.Worksheets.Item($Bank)
    foreach ($field in $Fields.Keys) {
        A-Assert ($avMapping.Contains($field)) "Unknown editable bank field: $field"
        $column = A-Column $Bank ([int]$avMapping[$field])
        $cell = $sheet.Cells.Item($Row,$column)
        if ([bool]$cell.Locked) {
            $diagnostic = A-LockDiagnostic $Bank $Row $column
            $script:avReport.lock_diagnostic = $diagnostic
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
function A-Stage([string]$Bank, [string]$Id, [System.Collections.IDictionary]$Fields) {
    A-Activate $Bank
    A-Fault $true
    try { A-Set $Bank (A-Row $Bank $Id) $Fields }
    finally { A-Fault $false }
    A-Sync
    A-EmptyQueue
}
function A-Canonical([string]$Id, [System.Collections.IDictionary]$Expected) {
    $actual = (A-Record $Id).fields
    foreach ($field in $Expected.Keys) {
        $property = $actual.PSObject.Properties[$field]
        A-Assert ($null -ne $property) "Missing canonical field $field."
        $value = $property.Value; $want = $Expected[$field]
        if ($null -eq $want) {
            A-Assert ($null -eq $value) "Canonical $field did not clear to null."
        } elseif ($want -is [bool]) {
            A-Assert (($value -is [bool]) -and ($value -eq $want)) "Canonical Boolean $field changed value/type."
        } else {
            $text = [string]$value
            if (($value -is [datetime] -or $value -is [datetimeoffset]) -and $field -in @('transaction_date','annotation_period')) {
                $format = if ($field -eq 'annotation_period') { 'yyyy-MM-dd' } else { "yyyy-MM-dd'T'HH:mm:ss.FFFFFFF" }
                $text = $value.ToString($format,[Globalization.CultureInfo]::InvariantCulture)
            }
            A-Assert ($text -ceq [string]$want) "Canonical field $field differs for receipt $Id."
        }
    }
}
function A-Grid([string]$Bank, [string]$Id, [System.Collections.IDictionary]$Expected) {
    $sheet = $Workbook.Worksheets.Item($Bank); $row = A-Row $Bank $Id
    $prototypeName = if ($Bank -ceq 'HUMO Card') { '_styleHumo' } else { '_styleCard' }
    $prototype = $Workbook.Worksheets.Item($prototypeName)
    foreach ($field in $Expected.Keys) {
        if ($field -eq 'excel_bank') { continue }
        $cell = $sheet.Cells.Item($row,(A-Column $Bank ([int]$avMapping[$field])))
        $value = $cell.Value2; $want = $Expected[$field]
        A-Assert (-not [bool]$cell.HasFormula) "$Bank $field became a formula."
        if ($field -in @('amount','balance_after')) {
            # Bank reference layouts intentionally display whole units. Preserve
            # that exact format independently of the cents stored in Value2.
            $originalFormat = [string]$prototype.Cells.Item(5,(A-Column $Bank ([int]$avMapping[$field]))).NumberFormat
            A-Assert ([string]$cell.NumberFormat -ceq $originalFormat) "$Bank $field changed the original bank number format."
        }
        if ($null -eq $want -or ($want -is [string] -and $want.Length -eq 0)) {
            A-Assert ([string]::IsNullOrEmpty([string]$value)) "$Bank $field is not blank after clear."
        } elseif ($field -in @('transaction_date','annotation_period')) {
            $date = [datetime]::Parse([string]$want,[Globalization.CultureInfo]::InvariantCulture)
            A-Assert ([Math]::Abs([double]$value-$date.ToOADate()) -lt 0.0000001) "$Bank $field changed local date/time."
        } elseif ($field -in @('amount','balance_after')) {
            $number = [double]::Parse([string]$want,[Globalization.CultureInfo]::InvariantCulture)
            if ($field -eq 'amount') { $number=[Math]::Abs($number) }
            A-Assert ([Math]::Abs([double]$value-$number) -lt 0.0000001) "$Bank $field lost cents/sign."
        } elseif ($field -in @('is_p2p','p2p_confirmed')) {
            A-Assert (-not [string]::IsNullOrEmpty([string]$value) -and [double]$value -eq [double][bool]$want) "$Bank $field is not the expected numeric 0/1."
        } elseif ($field -eq 'annotation_article_code') {
            A-Assert ([string]$value -ceq ([string]$want+'='+$avArticleLabels[[string]$want])) "$Bank article code=label rendering failed."
        } elseif ($field -eq 'transaction_type') {
            A-Assert (-not [string]::IsNullOrWhiteSpace([string]$value) -and [string]$value -cne [string]$want) "$Bank operation was not rendered as a decorated label."
        } else {
            A-Assert ([string]$value -ceq [string]$want) "$Bank $field changed Unicode/text/leading zeros."
        }
    }
    A-Assert ([double]$sheet.Rows.Item($row).RowHeight -eq 12) "$Bank receipt row lost its 12-point height."
}
function A-View([string]$Bank) {
    $sheet=A-Sheet $Bank; $window=$Workbook.Windows.Item(1)
    A-Assert ([string]$window.ActiveSheet.Name -ceq $Bank) 'Only capture a bank while its own view is active.'
    $selection=$window.RangeSelection; $active=$window.ActiveCell; $signature=@(); $panes=@()
    for ($i=1; $i -le [int]$selection.Areas.Count; $i++) {
        $area=$selection.Areas.Item($i)
        $signature+=[ordered]@{first_key=(A-Key $sheet ([int]$area.Row));
            last_key=(A-Key $sheet ([int]$area.Row+[int]$area.Rows.Count-1));
            column=[int]$area.Column;rows=[int]$area.Rows.Count;columns=[int]$area.Columns.Count}
    }
    for ($i=1; $i -le [int]$window.Panes.Count; $i++) {
        $pane=$window.Panes.Item($i)
        $panes+=[ordered]@{scroll_row=[int]$pane.ScrollRow;scroll_column=[int]$pane.ScrollColumn;
            top_key=(A-Key $sheet ([int]$pane.ScrollRow))}
    }
    return [pscustomobject][ordered]@{bank=$Bank;active=[string]$active.Address($false,$false);
        active_key=(A-Key $sheet ([int]$active.Row));active_column=[int]$active.Column;
        selection=[string]$selection.Address($false,$false);signature=($signature | ConvertTo-Json -Depth 10 -Compress);
        scroll_row=[int]$window.ScrollRow;scroll_column=[int]$window.ScrollColumn;
        top_key=(A-Key $sheet ([int]$window.ScrollRow));zoom=[int]$window.Zoom;
        freeze=[bool]$window.FreezePanes;split_row=[double]$window.SplitRow;split_column=[double]$window.SplitColumn;panes=$panes}
}
function A-Exact($Before,$After,[string]$Context) {
    foreach ($field in @('bank','active','active_key','active_column','selection','signature','scroll_row','scroll_column','top_key','zoom','freeze','split_row','split_column')) {
        A-Assert ([string]$Before.$field -ceq [string]$After.$field) "${Context}: changed $field."
    }
    A-Assert (($Before.panes | ConvertTo-Json -Compress) -ceq ($After.panes | ConvertTo-Json -Compress)) "${Context}: moved a pane."
}
function A-Anchor($Before,$After,[string]$Context) {
    foreach ($field in @('bank','active_key','active_column','signature','scroll_column','top_key','zoom','freeze','split_row','split_column')) {
        A-Assert ([string]$Before.$field -ceq [string]$After.$field) "${Context}: changed $field."
    }
    A-Assert (@($Before.panes).Count -eq @($After.panes).Count) "${Context}: changed pane count."
    for ($i=0; $i -lt @($Before.panes).Count; $i++) {
        A-Assert ([string]$Before.panes[$i].top_key -ceq [string]$After.panes[$i].top_key -and
            [int]$Before.panes[$i].scroll_column -eq [int]$After.panes[$i].scroll_column) "${Context}: changed a pane anchor."
    }
}
function A-FileHash {
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
# The visible All adapter intentionally shares CardXabar's physical columns.
$avAll = ('{"v":"\u0412\u0441\u0435 \u0442\u0440\u0430\u043d\u0437\u0430\u043a\u0446\u0438\u0438"}' | ConvertFrom-Json).v
function A-Sheet([string]$Name) { return $Workbook.Worksheets.Item($Name) }
function A-Last([string]$Name) {
    $sheet=A-Sheet $Name
    return [int]$sheet.Cells.Item($sheet.Rows.Count,(A-Column $Name 52)).End(-4162).Row
}
function A-Key($Sheet,[int]$Row) {
    if ($Row -lt (A-First ([string]$Sheet.Name))) { return 'header:'+$Row }
    $key=[string]$Sheet.Cells.Item($Row,(A-Column ([string]$Sheet.Name) 52)).Value2
    if ($key.Length -eq 0) { return 'blank:'+$Row }
    return $key
}
function A-Order([string]$Name) {
    $sheet=A-Sheet $Name; $keys=@()
    for ($r=(A-First $Name); $r -le (A-Last $Name); $r++) { $keys+=A-Key $sheet $r }
    return ,$keys
}
function A-SetupView {
    A-Activate $avAll
    $sheet=A-Sheet $avAll; $window=$Workbook.Windows.Item(1); $last=A-Last $avAll
    A-Assert ($last -ge 10) 'At least six receipts are required for All viewport anchors.'
    $window.FreezePanes=$false; $window.SplitRow=4.0; $window.SplitColumn=14.0; $window.FreezePanes=$true
    $window.Zoom=85
    $area1=$sheet.Range($sheet.Cells.Item($last-1,37),$sheet.Cells.Item($last,38))
    $area2=$sheet.Range($sheet.Cells.Item($last-1,40),$sheet.Cells.Item($last,40))
    $selection=$avApp.Union($area1,$area2); $selection.Select(); $sheet.Cells.Item($last,38).Activate()
    $window.ScrollRow=$last-2; $window.ScrollColumn=37
    A-Assert ([int]$window.RangeSelection.Areas.Count -eq 2) 'All multi-area selection was not retained.'
}
try {
    A-Assert ([string]$Session.test_marker -ceq 'TBSparcer isolated native Excel test' -and [string]$Session.api_url -ceq 'http://127.0.0.1:8767') 'Only the synthetic native fixture is allowed.'
    $testRoot=[IO.Path]::GetFullPath([string]$Session.test_root).TrimEnd('\','/')
    $bookPath=[IO.Path]::GetFullPath([string]$Workbook.FullName)
    A-Assert ($bookPath.StartsWith($testRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) 'Workbook is outside the owned test root.'
    A-Assert ($bookPath -ceq [IO.Path]::GetFullPath([string]$Session.workbook_path) -and [IO.Path]::GetExtension($bookPath) -ceq '.xlsm' -and -not [bool]$Workbook.ReadOnly) 'Expected the original writable synthetic XLSM.'
    $cfg=$Workbook.Worksheets.Item('_sync')
    A-Assert ([string]$cfg.Range('B1').Value2 -ceq [string]$Session.workbook_id -and [string]$cfg.Range('B5').Value2 -ceq [string]$Session.api_url) 'Workbook/session connection mismatch.'
    A-Assert ([string]$cfg.Range('B3').Value2 -ceq '_transactions' -and [int]$Workbook.Worksheets.Count -eq 9) 'Expected a nine-sheet workbook with a separate hidden core.'
    A-Assert ([int](A-Sheet '_transactions').Visible -eq 2) 'The core bridge must remain very hidden.'
    foreach ($name in @('CardXabar','HUMO Card',$avAll)) { A-Assert ([int](A-Sheet $name).Visible -eq -1) 'A requested transaction view is not visible.' }
    A-Assert ([int]$avApp.CutCopyMode -eq 0) 'Finish the current cut/copy before this fixture test.'
    $fixture=A-Server
    A-Assert ($fixture.synthetic -and [string]$fixture.instance_id -ceq [string]$Session.instance_id -and -not $fixture.faults.offline -and @($fixture.records).Count -ge 6) 'Expected the online synthetic fixture with at least six receipts.'
    A-Assert ([string](A-State).server_url -ceq [string]$Session.api_url) 'Durable state has a different server origin.'
    $avValidated=$true; $avReportPath=Join-Path $testRoot 'native-all-view-verification.json'
    $otherBooks=@()
    foreach ($other in $avApp.Workbooks) { if ([string]$other.FullName -cne $bookPath) { $otherBooks+=@{path=[string]$other.FullName;saved=[bool]$other.Saved} } }
    $avApp.EnableEvents=$true; A-EmptyQueue
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StartSync')
    A-Assert ([bool](Invoke-ExcelSyncMacro $Workbook 'BankViews.HasAllView')) 'The All adapter is not enabled.'
    $ids=@($fixture.records | Sort-Object {[int]$_.id} | ForEach-Object {[string]$_.id})
    $cardId=$ids[0]; $humoId=$ids[1]
    foreach ($pair in @(@{id=$cardId;bank='CardXabar'},@{id=$humoId;bank='HUMO Card'})) {
        [void](Invoke-ExcelSyncFixtureControl $Session POST '/__test__/server-edit' @{id=[int]$pair.id;fields=@{excel_bank=$pair.bank}})
    }
    A-Activate $avAll; A-Sync
    foreach ($name in @('CardXabar','HUMO Card',$avAll)) { A-Activate $name }
    A-EmptyQueue
    $refs=A-Sheet '_refs'; $avPerson=[string]$refs.Range('A2').Value2; $avArticle=[string]$refs.Range('C2').Value2
    $avArticleLabels=@{}
    for ($r=2; $r -le [int]$refs.Cells.Item($refs.Rows.Count,3).End(-4162).Row; $r++) { $avArticleLabels[[string]$refs.Cells.Item($r,3).Value2]=[string]$refs.Cells.Item($r,4).Value2 }
    A-Assert ($avPerson.Length -gt 0 -and $avArticle.Length -gt 0 -and $avArticleLabels[$avArticle].Length -gt 0) 'Synthetic reference lists are empty.'
    $unicode='{"operator":"\u041a\u0430\u0444\u0435 \u0401\u0436\u0438\u043a","details":"=\u041e\u0431\u0449\u0438\u0439 \u043b\u0438\u0441\u0442","receiver":"\u041e\u043b\u0451\u043d\u0430","raw":"\ud83d\udd34 Native All operation\n\u0412\u0442\u043e\u0440\u0430\u044f \u0441\u0442\u0440\u043e\u043a\u0430 \ud83d\udcb3"}' | ConvertFrom-Json
    A-Phase 'all_17_native_fields_existing_humo'
    $expected=[ordered]@{
        transaction_date='2026-08-31T23:59:58';transaction_type='DEBIT';operator_raw=$unicode.operator;
        application_mapped='native-All-app';amount='-9876.54';balance_after='-12.30';card_last_4='0007';
        is_p2p=$true;p2p_confirmed=$false;annotation_details=$unicode.details;annotation_person_code=$avPerson;
        annotation_period='2026-07-01';annotation_article_code=$avArticle;currency='USD';
        receiver_name=$unicode.receiver;receiver_card='0000123456789012';raw_message=$unicode.raw
    }
    $inputFields=[ordered]@{}; foreach ($field in $expected.Keys) { $inputFields[$field]=$expected[$field] }
    $inputFields.annotation_article_code=$avArticle+'='+$avArticleLabels[$avArticle]
    A-Stage $avAll $humoId $inputFields; A-Canonical $humoId $expected; A-Canonical $humoId @{excel_bank='HUMO Card'}; A-Grid $avAll $humoId $expected
    A-Activate 'HUMO Card'; A-Grid 'HUMO Card' $humoId $expected; A-Activate $avAll
    $avReport.checks+='All 17 real SheetChange edits round-trip with Unicode, dates, cents and article code=label; existing HUMO assignment is retained'

    A-Phase 'all_raw_operation_noop'
    $before=A-Server; $recordBefore=(A-Record $humoId) | ConvertTo-Json -Depth 50 -Compress
    $decorated=[string](A-Sheet $avAll).Cells.Item((A-Row $avAll $humoId),20).Value2
    A-Assert ($decorated -ceq ($unicode.raw -split [char]10)[0]) 'All lost the raw operation headline.'
    A-Fault $true
    try { A-Set $avAll (A-Row $avAll $humoId) @{transaction_type=$decorated}; A-EmptyQueue } finally { A-Fault $false }
    A-Sync; A-EmptyQueue
    A-Assert ([int](A-Server).counts.operations -eq [int]$before.counts.operations -and ((A-Record $humoId) | ConvertTo-Json -Depth 50 -Compress) -ceq $recordBefore) 'Unchanged All operation headline changed the server record/version or sent an operation.'
    $avReport.checks+='Re-entering the unchanged raw T headline is a durable-queue and server-operation no-op'

    A-Phase 'all_nullable_confirmation'
    foreach ($flags in @(@{p2p_confirmed=$true},@{is_p2p=$false},@{p2p_confirmed=$null})) {
        A-Stage $avAll $humoId $flags
        foreach ($field in $flags.Keys) { $expected[$field]=$flags[$field] }
        A-Canonical $humoId $expected; A-Grid $avAll $humoId $expected
    }
    $avReport.checks+='All p2p_confirmed false/true/null remains independent of is_p2p'

    A-Phase 'bank_edits_reflect_on_all'
    foreach ($pair in @(@{id=$cardId;bank='CardXabar'},@{id=$humoId;bank='HUMO Card'})) {
        $text='native-All-from-'+$pair.bank+'-'+[guid]::NewGuid().ToString('N')
        A-Stage $pair.bank $pair.id @{annotation_details=$text}
        A-Activate $avAll; A-Grid $avAll $pair.id @{annotation_details=$text}
        A-Canonical $pair.id @{excel_bank=$pair.bank;annotation_details=$text}
    }
    $avReport.checks+='Edits on either bank appear in All on activation and retain the canonical bank'

    A-Phase 'all_native_create_without_bank'
    A-Activate $avAll
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.AddReceipt')
    A-Assert ([string]$avApp.ActiveSheet.Name -ceq $avAll -and [int]$Workbook.Windows.Item(1).ActiveCell.Row -eq (A-Last $avAll)+1) 'AddReceipt left the All input row.'
    $sheet=A-Sheet $avAll; $draftRow=(A-Last $avAll)+1; $marker='native-All-create-'+[guid]::NewGuid().ToString('N'); $before=A-Server
    $earliest=[double]::PositiveInfinity
    for ($r=5; $r -lt $draftRow; $r++) { $earliest=[Math]::Min($earliest,[double]$sheet.Cells.Item($r,17).Value2) }
    $draftDate=[datetime]::FromOADate($earliest).Date.AddDays(-1).AddHours(12)
    A-Fault $true
    try {
        A-Set $avAll $draftRow @{operator_raw=$marker}
        $draftUuid=[string]$sheet.Cells.Item($draftRow,52).Value2; $parsed=[guid]::Empty
        A-Assert ([guid]::TryParse($draftUuid,[ref]$parsed) -and [string]::IsNullOrEmpty([string]$sheet.Cells.Item($draftRow,51).Value2)) 'All incomplete draft identity is invalid.'
        $intent=(A-State).intents.PSObject.Properties['u:'+$draftUuid]
        A-Assert ($null -ne $intent -and [string]$intent.Value.kind -ceq 'draft') 'All input did not create a durable draft.'
        A-Assert ($null -eq $intent.Value.fields.excel_bank) 'All draft must not invent an excel_bank.'
        A-Set $avAll $draftRow ([ordered]@{transaction_date=$draftDate;transaction_type='DEBIT';card_last_4='0042';currency='USD';amount='42.37'})
        A-Assert ([string]$sheet.Cells.Item($draftRow,52).Value2 -ceq $draftUuid -and [int](A-Server).counts.transactions -eq [int]$before.counts.transactions) 'Offline All draft changed UUID or reached the server.'
    } finally { A-Fault $false }
    A-Sync; A-EmptyQueue; $after=A-Server
    $created=@($after.records | Where-Object {[string]$_.fields.operator_raw -ceq $marker})
    A-Assert ($created.Count -eq 1 -and [int]$after.counts.transactions -eq [int]$before.counts.transactions+1 -and [int]$after.counts.operations -eq [int]$before.counts.operations+1) 'All draft must commit exactly one create.'
    $createdId=[string]$created[0].id; $createdUuid=[string]$created[0].uuid
    A-Canonical $createdId @{excel_bank=$null;p2p_confirmed=$null;amount='-42.37'}
    A-Assert ((A-Row $avAll $createdId) -eq $draftRow) 'All creation acknowledgement moved the draft.'
    A-Activate 'CardXabar'; [void](A-Row 'CardXabar' $createdId); A-Activate $avAll
    $avReport.checks+='All AddReceipt stays on its input; an offline native draft creates once with null bank and retains its All row, also appearing in the fallback Card view'

    A-Phase 'all_inactive_structural_delete_uuid_anchor'
    A-SetupView; $beforeView=A-View $avAll; $beforeOrder=A-Order $avAll; $deleteId=$null
    for ($r=5; $r -lt [int]$beforeView.scroll_row; $r++) {
        $candidate=[string](A-Sheet $avAll).Cells.Item($r,51).Value2
        if ($candidate -notin @($cardId,$humoId,$createdId)) { $deleteId=$candidate; break }
    }
    A-Assert (-not [string]::IsNullOrEmpty($deleteId)) 'No disposable synthetic receipt remains above the All selection.'
    A-Activate 'CardXabar'
    [void](Invoke-ExcelSyncFixtureControl $Session DELETE ('/__test__/record/'+$deleteId)); A-Sync; A-EmptyQueue
    A-Assert (((A-Order $avAll)-join ',') -ceq ($beforeOrder-join ',') -and [bool](A-Sheet $avAll).Cells.Item(5,37).Locked) 'Inactive All was compacted or remained editable before activation.'
    A-Assert ([string]$avApp.ActiveSheet.Name -ceq 'CardXabar') 'Inactive All deletion stole focus.'
    A-Activate $avAll; $afterView=A-View $avAll; A-Anchor $beforeView $afterView 'Deferred All deletion'
    A-Assert ([int]$afterView.scroll_row -eq [int]$beforeView.scroll_row-1 -and -not [bool](A-Sheet $avAll).Cells.Item(5,37).Locked) 'All did not compact and unlock after activation.'
    $avReport.checks+='Inactive All defers structural deletion and locks; activation preserves active UUID, multi-area selection, pane anchors and zoom'

    A-Phase 'all_sort_preserves_uuid_and_bank_orders'
    foreach ($name in @('CardXabar','HUMO Card',$avAll)) { A-Activate $name }
    $cardOrder=A-Order 'CardXabar'; $humoOrder=A-Order 'HUMO Card'
    $sheet=A-Sheet $avAll; $sheet.Cells.Item((A-Row $avAll $createdId),37).Select()
    [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.SortByDate')
    A-Assert ((A-View $avAll).active_key -ceq $createdUuid -and (A-Row $avAll $createdId) -eq 5) 'All sort did not move the earliest-dated draft first while preserving its selected UUID.'
    $previous=[double]::NegativeInfinity
    for ($r=5; $r -le (A-Last $avAll); $r++) { $date=[double]$sheet.Cells.Item($r,17).Value2; A-Assert ($date -ge $previous) 'All dates are not sorted ascending.'; $previous=$date }
    A-Assert (((A-Order 'CardXabar')-join ',') -ceq ($cardOrder-join ',') -and ((A-Order 'HUMO Card')-join ',') -ceq ($humoOrder-join ',')) 'All sorting changed a bank order.'
    $avReport.checks+='All SortByDate preserves the selected UUID and both independent bank orders'

    A-Phase 'all_noop_view_saved_and_file'
    A-SetupView; $sheet=A-Sheet $avAll
    $sheet.Unprotect('tbs-layout')
    try { $sheet.Rows.Item(6).Hidden=$true } finally { [void](Invoke-ExcelSyncMacro $Workbook 'BankViews.SetEditable' @($true)) }
    $Workbook.Save()
    A-Assert ([bool]$Workbook.Saved) 'The owned All test workbook did not save.'
    $beforeView=A-View $avAll; $beforeOrder=A-Order $avAll; $hash=A-FileHash; $writeTime=(Get-Item -LiteralPath $bookPath).LastWriteTimeUtc.Ticks
    for ($cycle=1; $cycle -le 2; $cycle++) {
        A-Sync; A-EmptyQueue; A-Exact $beforeView (A-View $avAll) 'All no-op'
        A-Assert ([bool]$Workbook.Saved -and (A-FileHash) -ceq $hash -and (Get-Item -LiteralPath $bookPath).LastWriteTimeUtc.Ticks -eq $writeTime) 'An All no-op rewrote or dirtied the workbook.'
        A-Assert ([bool]$sheet.Rows.Item(6).Hidden -and ((A-Order $avAll)-join ',') -ceq ($beforeOrder-join ',')) 'All no-op changed hidden rows or order.'
    }
    $avReport.checks+='Two All no-op cycles preserve Saved, file SHA256/write time, order, hidden row, selection, panes and zoom'

    A-Phase 'all_exact_membership'
    foreach ($name in @('CardXabar','HUMO Card',$avAll)) { A-Activate $name }
    $server=A-Server; $serverKeys=@($server.records | ForEach-Object {[string]$_.uuid}); $allKeys=A-Order $avAll
    $bankKeys=@((A-Order 'CardXabar')+(A-Order 'HUMO Card'))
    A-Assert ($allKeys.Count -eq $serverKeys.Count -and @($allKeys | Sort-Object -Unique).Count -eq $serverKeys.Count -and (($allKeys | Sort-Object)-join ',') -ceq (($serverKeys | Sort-Object)-join ',')) 'All must contain every canonical UUID exactly once.'
    A-Assert ($bankKeys.Count -eq $serverKeys.Count -and (($bankKeys | Sort-Object)-join ',') -ceq (($serverKeys | Sort-Object)-join ',')) 'The two bank views must independently partition canonical UUIDs exactly once.'
    foreach ($other in $otherBooks) {
        $matches=@($avApp.Workbooks | Where-Object {[string]$_.FullName -ceq [string]$other.path})
        A-Assert ($matches.Count -eq 1 -and [bool]$matches[0].Saved -eq [bool]$other.saved) 'An unrelated workbook was closed or dirtied.'
    }
    $avReport.checks+='All contains every receipt exactly once; the bank union independently contains the same UUID set; unrelated workbooks are untouched'
    $avReport.created=@{id=$createdId;uuid=$createdUuid}; $avReport.deleted_id=$deleteId
    A-EmptyQueue; $avReport.passed=$true; A-Phase 'complete'
} catch {
    $avReport.error=$_.Exception.Message -replace '(?i)excel_[A-Za-z0-9_-]{20,}','[redacted]'
    $avReport.error_line=$_.InvocationInfo.ScriptLineNumber
    throw
} finally {
    if ($avValidated) {
        try { A-Fault $false } catch { $avReport.fixture_cleanup_error=$_.Exception.Message }
        try { [void](Invoke-ExcelSyncMacro $Workbook 'ExcelSync.StopSync') } catch { $avReport.stop_error=$_.Exception.Message }
    }
    $avApp.EnableEvents=$avOriginalEvents
    if ($avReportPath) { $avReport | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $avReportPath -Encoding UTF8 }
}
[pscustomobject]$avReport
