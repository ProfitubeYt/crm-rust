Attribute VB_Name = "BankViews"
Option Explicit

' Bank sheets are editable views of one durable, UUID-keyed synchronization state.
' The hidden legacy grid remains the input bridge; no second queue is introduced.
Private Const PASSWORD As String = "tbs-layout"
Private mRendering As Boolean
Private mStylesReady As Boolean
Private mDeferred As Boolean
Private mDeferredBanks As Object
Private mArticles As Object
Private mEditableState As Object

Private Function Txt(ByVal value As Variant) As String
    If IsNull(value) Or IsEmpty(value) Then Exit Function
    If Not IsError(value) Then Txt = CStr(value)
End Function

Private Function Field(ByVal fields As Object, ByVal name As String) As Variant
    Field = Null
    If fields.Exists(name) Then Field = fields(name)
End Function

Private Function U(ByVal text As String) As String
    Static cache As Object
    If cache Is Nothing Then Set cache = Dict()
    If Not cache.Exists(text) Then cache.Add text, JsonParse("{""v"":""" & text & """}")("v")
    U = CStr(cache(text))
End Function

Private Function NumberValue(ByVal value As Variant) As Variant
    If VarType(value) = vbString Then
        NumberValue = CDec(Replace$(CStr(value), ".", Mid$(CStr(1.5), 2, 1)))
    Else
        NumberValue = CDec(value)
    End If
End Function

Public Function Enabled() As Boolean
    Dim sheet As Worksheet
    On Error GoTo Missing
    Set sheet = ThisWorkbook.Worksheets("CardXabar")
    Set sheet = ThisWorkbook.Worksheets("HUMO Card")
    Enabled = True
Missing:
End Function

Public Function AllName() As String
    AllName = U("\u0412\u0441\u0435 \u0442\u0440\u0430\u043d\u0437\u0430\u043a\u0446\u0438\u0438")
End Function

Private Function HasAllView() As Boolean
    Dim sheet As Worksheet
    On Error GoTo Missing
    ' Older two-bank books use this name for their hidden input bridge.
    If ExcelSync.DataSheetName() = AllName() Then Exit Function
    Set sheet = ThisWorkbook.Worksheets(AllName())
    HasAllView = True
Missing:
End Function

Private Function ViewNames() As Variant
    If HasAllView() Then
        ViewNames = Array("CardXabar", "HUMO Card", AllName())
    Else
        ViewNames = Array("CardXabar", "HUMO Card")
    End If
End Function

Public Function Views() As Variant
    Views = ViewNames()
End Function

Public Function IsBankSheet(ByVal name As String) As Boolean
    If Not Enabled() Then Exit Function
    IsBankSheet = (name = "CardXabar" Or name = "HUMO Card")
    If name = AllName() Then IsBankSheet = HasAllView()
End Function

Private Function FirstRow(ByVal bank As String) As Long
    If bank = "HUMO Card" Then FirstRow = 4 Else FirstRow = 5
End Function

Public Function PhysicalColumn(ByVal bank As String, ByVal core As Long) As Long
    If bank <> "HUMO Card" Then PhysicalColumn = core: Exit Function
    Select Case core
        Case 15 To 25: PhysicalColumn = core
        Case 26: PhysicalColumn = 32
        Case 32: PhysicalColumn = 33
        Case 33 To 53: PhysicalColumn = core + 7
    End Select
End Function

Private Function CoreColumn(ByVal bank As String, ByVal physical As Long) As Long
    Dim col As Long
    For col = 15 To 53
        If PhysicalColumn(bank, col) = physical Then CoreColumn = col: Exit Function
    Next
End Function

Private Function InputColumn(ByVal bank As String, ByVal physical As Long) As Long
    Dim col As Long
    col = CoreColumn(bank, physical)
    Select Case col
        Case 17, 20, 21, 23, 26, 33, 34, 35, 37 To 44: InputColumn = col
        Case 36: InputColumn = 54 ' Nullable independent p2p confirmation.
    End Select
End Function

Private Function StyleName(ByVal bank As String) As String
    If bank = "HUMO Card" Then StyleName = "_styleHumo" Else StyleName = "_styleCard"
End Function

Private Function StylePrefix(ByVal bank As String) As String
    If bank = "HUMO Card" Then StylePrefix = "_TBSbank_20260915_h" Else StylePrefix = "_TBSbank_20260915_c"
End Function

Private Function BankFor(ByVal record As Object, ByVal fields As Object) As String
    Dim assigned As String, source As String
    assigned = Txt(Field(fields, "excel_bank"))
    If assigned = "CardXabar" Or assigned = "HUMO Card" Then BankFor = assigned: Exit Function
    If record("metadata").Exists("source_display") Then source = Txt(record("metadata")("source_display"))
    If InStr(1, source, "HUMO", vbTextCompare) > 0 Then BankFor = "HUMO Card" Else BankFor = "CardXabar"
End Function

Private Function KeyAt(ByVal sheet As Worksheet, ByVal row As Long) As String
    Dim uuid As String, id As String
    If row < FirstRow(sheet.Name) Then Exit Function
    uuid = Txt(sheet.Cells(row, PhysicalColumn(sheet.Name, 52)).Value2)
    If Len(uuid) > 0 Then KeyAt = "u:" & uuid: Exit Function
    id = Txt(sheet.Cells(row, PhysicalColumn(sheet.Name, 51)).Value2)
    If Len(id) > 0 Then KeyAt = "i:" & id
End Function

Private Function ArrayKey(ByRef keys As Variant, ByVal row As Long) As String
    If Len(Txt(keys(row, 2))) > 0 Then
        ArrayKey = "u:" & Txt(keys(row, 2))
    ElseIf Len(Txt(keys(row, 1))) > 0 Then
        ArrayKey = "i:" & Txt(keys(row, 1))
    End If
End Function

Private Function ReadKeys(ByVal sheet As Worksheet, ByVal first As Long, ByVal last As Long) As Variant
    Dim col As Long
    col = PhysicalColumn(sheet.Name, 51)
    ReadKeys = sheet.Range(sheet.Cells(first, col), sheet.Cells(last, col + 1)).Value2
End Function

Private Function KeyInCore(ByRef core As Variant, ByVal row As Long) As String
    If Len(Txt(core(row, 38))) > 0 Then
        KeyInCore = "u:" & Txt(core(row, 38))
    ElseIf Len(Txt(core(row, 37))) > 0 Then
        KeyInCore = "i:" & Txt(core(row, 37))
    End If
End Function

Private Sub ReadArticles()
    Dim refs As Worksheet, last As Long, values As Variant, row As Long
    Set mArticles = Dict(): Set refs = ThisWorkbook.Worksheets("_refs")
    last = refs.Cells(refs.Rows.Count, 3).End(xlUp).row
    If last < 2 Then Exit Sub
    values = refs.Range("C2:D" & last).Value2
    For row = 1 To UBound(values, 1)
        If Len(Txt(values(row, 1))) > 0 Then mArticles(Txt(values(row, 1))) = Txt(values(row, 2))
    Next
End Sub

Private Function ArticleDisplay(ByVal value As Variant) As String
    Dim code As String
    code = Txt(value): ArticleDisplay = code
    If mArticles.Exists(code) Then
        If Len(Txt(mArticles(code))) > 0 Then ArticleDisplay = code & "=" & Txt(mArticles(code))
    End If
End Function

Private Function ArticleInput(ByVal value As Variant, ByVal original As Variant) As Variant
    Dim text As String, separator As Long, code As String
    text = Txt(value): separator = InStr(1, text, "=", vbBinaryCompare)
    If separator > 1 Then
        code = Left$(text, separator - 1)
        If mArticles.Exists(code) Then ArticleInput = code: Exit Function
        Err.Raise 5, , "Choose an article from the list."
    End If
    If Len(text) = 0 Then ArticleInput = Empty: Exit Function
    If text = Txt(original) Or mArticles.Exists(text) Then ArticleInput = value: Exit Function
    Err.Raise 5, , "Choose an article from the list."
End Function

Private Function RawLine(ByVal raw As String, ByVal marker1 As String, ByVal marker2 As String) As String
    Dim lines As Variant, line As Variant, text As String
    lines = Split(Replace$(Replace$(raw, vbCrLf, vbLf), vbCr, vbLf), vbLf)
    For Each line In lines
        text = Trim$(CStr(line))
        If Left$(text, Len(marker1)) = marker1 Or Left$(text, Len(marker2)) = marker2 Then RawLine = text: Exit Function
    Next
End Function

Private Function GroupMoney(ByVal value As Variant, ByVal bank As String) As String
    Dim text As String, whole As String, fraction As String, pos As Long, grouped As String, decimalSeparator As String, thousands As String
    If IsNull(value) Or IsEmpty(value) Then Exit Function
    If Len(Txt(value)) = 0 Then Exit Function
    text = Format$(Abs(NumberValue(value)), "0.00")
    pos = Len(text) - 2: whole = Left$(text, pos - 1): fraction = Right$(text, 2)
    If bank = "HUMO Card" Then decimalSeparator = ",": thousands = "." Else decimalSeparator = ".": thousands = " "
    Do While Len(whole) > 3
        grouped = thousands & Right$(whole, 3) & grouped: whole = Left$(whole, Len(whole) - 3)
    Loop
    GroupMoney = whole & grouped & decimalSeparator & fraction
End Function

Private Function MoneyText(ByVal fields As Object, ByVal bank As String, ByVal balance As Boolean) As String
    Dim raw As String, value As Variant, marker As String, currencyCode As String, sign As String
    raw = Txt(Field(fields, "raw_message")): currencyCode = Txt(Field(fields, "currency"))
    If balance Then
        MoneyText = RawLine(raw, U("\ud83d\udcb5"), U("\ud83d\udcb0"))
        value = Field(fields, "balance_after")
        If bank = "HUMO Card" Then marker = U("\ud83d\udcb0") Else marker = U("\ud83d\udcb5")
    Else
        MoneyText = RawLine(raw, U("\u2796"), U("\u2795"))
        value = Field(fields, "amount")
        Select Case Txt(Field(fields, "transaction_type"))
            Case "DEBIT", "CONVERSION": marker = U("\u2796")
            Case Else: marker = U("\u2795")
        End Select
    End If
    ' These columns retain the bank's original message, including its balance currency.
    ' The separate numeric cells always contain the exact current canonical values.
    If Len(MoneyText) > 0 Or IsNull(value) Then Exit Function
    If Len(currencyCode) = 0 Then currencyCode = "UZS"
    If balance Then
        If NumberValue(value) < 0 Then sign = "-"
    End If
    MoneyText = marker & " " & sign & GroupMoney(value, bank) & " " & currencyCode
End Function

Private Sub DigitGroups(ByRef desired() As Variant, ByVal row As Long, ByVal physical As Long, ByVal amount As Variant)
    Dim whole As String, groups As Variant, text As String, remaining As String, count As Long, i As Long
    If IsNull(amount) Or IsEmpty(amount) Then Exit Sub
    If Len(Txt(amount)) = 0 Then Exit Sub
    If NumberValue(amount) = 0 Then
        For i = 0 To 5: desired(row, physical - 14 + i) = 0: Next
        Exit Sub
    End If
    whole = CStr(Fix(Abs(NumberValue(amount)))): remaining = whole
    Do While Len(remaining) > 3
        text = "." & Right$(remaining, 3) & text: remaining = Left$(remaining, Len(remaining) - 3)
    Loop
    text = remaining & text: groups = Split(text, "."): count = UBound(groups) + 1
    If physical = 26 Then
        desired(row, physical - 14) = " " & text: desired(row, physical - 13) = Len(text) + 1
    Else
        desired(row, physical - 14) = text: desired(row, physical - 13) = Len(text)
    End If
    For i = 0 To 3
        If i < count Then
            If i = count - 1 Then desired(row, physical - 9 - i) = CLng(groups(count - 1 - i)) Else desired(row, physical - 9 - i) = CStr(groups(count - 1 - i))
        Else
            desired(row, physical - 9 - i) = 0
        End If
    Next
End Sub

Private Function OrderedRows(ByVal bank As String, ByVal coreRows As Object, ByVal records As Object, ByVal fieldsByKey As Object) As Collection
    Dim ordered As New Collection, seen As Object, sheet As Worksheet, last As Long, row As Long, key As String, item As Variant
    Dim keys As Variant
    Set seen = Dict(): Set sheet = ThisWorkbook.Worksheets(bank)
    last = sheet.Cells(sheet.Rows.Count, PhysicalColumn(bank, 52)).End(xlUp).row
    If last >= FirstRow(bank) Then keys = ReadKeys(sheet, FirstRow(bank), last)
    For row = FirstRow(bank) To last
        key = ExcelSync.ViewDisplayKey(ArrayKey(keys, row - FirstRow(bank) + 1))
        If coreRows.Exists(key) Then
            If bank = AllName() Or BankFor(records(key), fieldsByKey(key)) = bank Then
                If Not seen.Exists(key) Then ordered.Add key: seen.Add key, True
            End If
        End If
    Next
    For Each item In coreRows.Keys
        key = CStr(item)
        If Not seen.Exists(key) Then
            If bank = AllName() Or BankFor(records(key), fieldsByKey(key)) = bank Then ordered.Add key
        End If
    Next
    Set OrderedRows = ordered
End Function

Private Function Same(ByVal a As Variant, ByVal b As Variant) As Boolean
    If IsError(a) Or IsError(b) Then Exit Function
    If Len(Txt(a)) = 0 And Len(Txt(b)) = 0 Then Same = True: Exit Function
    ' Excel's Empty cell can coerce to numeric zero. A first render must still
    ' write actual zero values (false P2P, zero balance, and numeric helpers).
    If IsNull(a) Or IsEmpty(a) Or IsNull(b) Or IsEmpty(b) Then Exit Function
    If VarType(a) = vbString Or VarType(b) = vbString Then
        Same = (StrComp(Txt(a), Txt(b), vbBinaryCompare) = 0)
    ElseIf IsNumeric(a) And IsNumeric(b) Then
        Same = (CDbl(a) = CDbl(b))
    Else
        Same = (Txt(a) = Txt(b))
    End If
End Function

Private Function CaptureView(ByVal sheet As Worksheet) As Collection
    Dim result As New Collection, win As Window, view As Object, part As Object, pane As Pane, panes As Collection, areas As Collection, a As Range, detail As Object
    Dim keys As Collection, row As Long, lastData As Long, windowIndex As Long, paneIndex As Long, areaIndex As Long
    lastData = sheet.Cells(sheet.Rows.Count, PhysicalColumn(sheet.Name, 52)).End(xlUp).row
    For windowIndex = 1 To ThisWorkbook.Windows.Count
        Set win = ThisWorkbook.Windows.Item(windowIndex)
        If win.ActiveSheet.Name = sheet.Name Then
            Set view = Dict(): Set panes = New Collection: Set areas = New Collection
            view.Add "window", win: view.Add "active", win.ActiveCell.Address
            view.Add "active_key", KeyAt(sheet, win.ActiveCell.row)
            For areaIndex = 1 To win.RangeSelection.Areas.Count
                Set a = win.RangeSelection.Areas.Item(areaIndex)
                Set detail = Dict(): detail.Add "address", a.Address
                Set keys = New Collection
                If a.row >= FirstRow(sheet.Name) And a.row + a.Rows.Count - 1 <= lastData Then
                    For row = a.row To a.row + a.Rows.Count - 1: keys.Add KeyAt(sheet, row): Next
                End If
                detail.Add "keys", keys
                areas.Add detail
            Next
            view.Add "areas", areas
            For paneIndex = 1 To win.Panes.Count
                Set pane = win.Panes.Item(paneIndex)
                Set part = Dict(): part.Add "pane", pane: part.Add "row", pane.ScrollRow: part.Add "column", pane.ScrollColumn
                part.Add "key", KeyAt(sheet, pane.ScrollRow): panes.Add part
            Next
            view.Add "panes", panes: result.Add view
        End If
    Next
    Set CaptureView = result
End Function

Private Sub RestoreView(ByVal sheet As Worksheet, ByVal views As Collection, ByVal positions As Object)
    Dim view As Variant, part As Variant, detail As Variant, win As Window, pane As Pane, key As String, first As Long, last As Long, target As Long
    Dim selection As Range, area As Range, item As Range, active As Range, isActive As Boolean, recordKey As Variant
    If views Is Nothing Then Exit Sub
    For Each view In views
        Set win = view("window"): isActive = False
        If Not Application.ActiveWindow Is Nothing Then isActive = (win.Hwnd = Application.ActiveWindow.Hwnd)
        If isActive Then
            Set selection = Nothing
            For Each detail In view("areas")
                Set area = sheet.Range(detail("address")): Set item = area
                first = 0: last = 0
                For Each recordKey In detail("keys")
                    key = ExcelSync.ViewDisplayKey(CStr(recordKey))
                    If positions.Exists(key) Then
                        target = CLng(positions(key))
                        If first = 0 Or target < first Then first = target
                        If target > last Then last = target
                    End If
                Next
                If first > 0 And last >= first Then Set item = sheet.Range(sheet.Cells(first, area.Column), sheet.Cells(last, area.Column + area.Columns.Count - 1))
                If selection Is Nothing Then Set selection = item Else Set selection = Union(selection, item)
            Next
            If Not selection Is Nothing Then
                If selection.Address <> win.RangeSelection.Address Then selection.Select
            End If
            Set active = sheet.Range(view("active"))
            key = ExcelSync.ViewDisplayKey(Txt(view("active_key")))
            If positions.Exists(key) Then Set active = sheet.Cells(CLng(positions(key)), active.Column)
            If active.Address <> win.ActiveCell.Address Then active.Activate
        End If
        For Each part In view("panes")
            Set pane = part("pane"): target = CLng(part("row"))
            key = ExcelSync.ViewDisplayKey(Txt(part("key"))): If positions.Exists(key) Then target = CLng(positions(key))
            If pane.ScrollRow <> target Then pane.ScrollRow = target
            If pane.ScrollColumn <> CLng(part("column")) Then pane.ScrollColumn = CLng(part("column"))
        Next
    Next
End Sub

Public Sub Render()
    Dim coreSheet As Worksheet, core As Variant, last As Long, row As Long, key As String, records As Object, fieldsByKey As Object, coreRows As Object
    Dim record As Object, fields As Object, bank As Variant, order As Collection, offset As Long, events As Boolean, screen As Boolean, message As String, phase As String
    If Not Enabled() Or mRendering Then Exit Sub
    If Application.CutCopyMode <> False Then Exit Sub
    Set records = ExcelSync.ViewRecords(): If records Is Nothing Then Exit Sub
    If mDeferredBanks Is Nothing Then Set mDeferredBanks = Dict()
    mRendering = True: events = Application.EnableEvents: screen = Application.ScreenUpdating
    On Error GoTo Failed
    Application.EnableEvents = False: Application.ScreenUpdating = False
    phase = "styles"
    If Not mStylesReady Then
        ExcelSync.ViewEnsureColumnStyles "_styleCard", StylePrefix("CardXabar"), 53
        ExcelSync.ViewEnsureColumnStyles "_styleHumo", StylePrefix("HUMO Card"), 60
        mStylesReady = True
    End If
    phase = "articles": ReadArticles
    Set coreRows = Dict(): Set fieldsByKey = Dict(): Set coreSheet = ExcelSync.ViewCoreSheet()
    last = coreSheet.Cells(coreSheet.Rows.Count, 52).End(xlUp).row
    If last < 5 Then last = 5
    core = coreSheet.Range("O5:BA" & last).Value2
    phase = "core records"
    For row = 1 To UBound(core, 1)
        key = KeyInCore(core, row)
        If records.Exists(key) Then
            coreRows.Add key, row: fieldsByKey.Add key, ExcelSync.ViewFields(key)
        End If
    Next
    mDeferred = False: offset = 0
    For Each bank In ViewNames()
        phase = "order " & CStr(bank)
        Set order = OrderedRows(CStr(bank), coreRows, records, fieldsByKey)
        phase = "render " & CStr(bank)
        If CStr(bank) = AllName() Then offset = 0
        RenderBank CStr(bank), order, offset, core, coreRows, fieldsByKey
        offset = offset + order.Count
    Next
Finished:
    Application.EnableEvents = events: Application.ScreenUpdating = screen: mRendering = False
    If Len(message) > 0 Then Err.Raise 5, , message
    Exit Sub
Failed:
    message = "Cannot update bank sheets (" & phase & "): " & Err.Description
    On Error Resume Next
    SetEditable False
    On Error GoTo 0
    GoTo Finished
End Sub

Private Sub RenderBank(ByVal bank As String, ByVal order As Collection, ByVal offset As Long, ByRef core As Variant, ByVal coreRows As Object, ByVal fieldsByKey As Object)
    Dim sheet As Worksheet, row As Long, col As Long, sourceRow As Long, physical As Long, first As Long, last As Long, oldLast As Long, lastCol As Long
    Dim desired() As Variant, current As Variant, key As String, fields As Object, positions As Object, stamps As Object, stamp As String
    Dim changed As Object, formulaCells As Object, formulaState As Variant, cell As Range, formulaRange As Range, block() As Variant
    Dim structural As Boolean, changedRow As Variant, actualRow As Long, startCol As Long, endCol As Long, i As Long, views As Collection, message As String, existingKey As String, hasFormulas As Boolean, wasHidden As Boolean, phase As String
    Dim differences() As Boolean, formatStart As Long, formatEnd As Long, needsFormat As Boolean
    On Error GoTo Failed
    phase = "prepare"
    Set sheet = ThisWorkbook.Worksheets(bank): first = FirstRow(bank): lastCol = PhysicalColumn(bank, 53)
    oldLast = sheet.Cells(sheet.Rows.Count, PhysicalColumn(bank, 52)).End(xlUp).row
    last = first + order.Count: If oldLast + 1 > last Then last = oldLast + 1
    ReDim desired(1 To last - first + 1, 1 To lastCol - 14)
    ReDim differences(1 To last - first + 1, 1 To lastCol - 14)
    Set positions = Dict(): Set stamps = Dict(): Set changed = Dict(): Set formulaCells = Dict()
    For row = 1 To order.Count
        phase = "values row " & row
        key = CStr(order(row)): sourceRow = CLng(coreRows(key)): Set fields = fieldsByKey(key)
        positions(key) = first + row - 1
        For col = 15 To 53
            physical = PhysicalColumn(bank, col)
            If physical > 0 Then desired(row, physical - 14) = core(sourceRow, col - 14)
        Next
        desired(row, 1) = row: desired(row, 2) = offset + row
        stamp = Txt(Field(fields, "transaction_date"))
        If Len(stamp) > 0 Then
            If Not stamps.Exists(stamp) Then stamps.Add stamp, 0
            desired(row, 4) = CDbl(core(sourceRow, 3)) + CDbl(stamps(stamp)) * 0.0001
            stamps(stamp) = CLng(stamps(stamp)) + 1
        End If
        Select Case Txt(Field(fields, "transaction_type"))
            Case "DEBIT", "CONVERSION": desired(row, 10) = "-"
            Case "CREDIT", "REVERSAL": desired(row, 10) = "+"
            Case Else: desired(row, 10) = Empty
        End Select
        desired(row, PhysicalColumn(bank, 25) - 14) = MoneyText(fields, bank, False)
        desired(row, PhysicalColumn(bank, 32) - 14) = MoneyText(fields, bank, True)
        desired(row, PhysicalColumn(bank, 40) - 14) = ArticleDisplay(Field(fields, "annotation_article_code"))
        If Not IsNull(Field(fields, "p2p_confirmed")) Then desired(row, PhysicalColumn(bank, 36) - 14) = Abs(CInt(Field(fields, "p2p_confirmed"))) Else desired(row, PhysicalColumn(bank, 36) - 14) = Empty
        If bank = "HUMO Card" Then
            DigitGroups desired, row, 26, Field(fields, "amount")
            DigitGroups desired, row, 34, Field(fields, "balance_after")
        End If
    Next
    desired(order.Count + 1, PhysicalColumn(bank, 50) - 14) = U("\u041d\u043e\u0432\u044b\u0439 \u0447\u0435\u043a: \u0437\u0430\u043f\u043e\u043b\u043d\u0438\u0442\u0435 \u0441\u0442\u0440\u043e\u043a\u0443")
    phase = "compare"
    current = sheet.Range(sheet.Cells(first, 15), sheet.Cells(last, lastCol)).Value2
    formulaState = sheet.Range(sheet.Cells(first, 15), sheet.Cells(last, lastCol)).HasFormula
    hasFormulas = IsNull(formulaState)
    If Not hasFormulas Then hasFormulas = CBool(formulaState)
    If hasFormulas Then
        Set formulaRange = sheet.Range(sheet.Cells(first, 15), sheet.Cells(last, lastCol)).SpecialCells(xlCellTypeFormulas)
        For Each cell In formulaRange.Cells: formulaCells(CStr(cell.row - first + 1) & ":" & CStr(cell.Column - 14)) = True: Next
    End If
    For row = 1 To UBound(desired, 1)
        For col = 1 To UBound(desired, 2)
            differences(row, col) = Not Same(current(row, col), desired(row, col))
            If hasFormulas Then
                If formulaCells.Exists(CStr(row) & ":" & CStr(col)) Then differences(row, col) = True
            End If
            If differences(row, col) Then changed(CStr(row)) = True
        Next
        If Txt(current(row, PhysicalColumn(bank, 52) - 14)) <> Txt(desired(row, PhysicalColumn(bank, 52) - 14)) Then structural = True
    Next
    If changed.Count = 0 Then
        If mDeferredBanks.Exists(bank) Then mDeferredBanks.Remove bank
        Exit Sub
    End If
    If structural Then
        If Not ActiveWorkbook Is ThisWorkbook Then
            mDeferred = True: mDeferredBanks(bank) = True: ProtectBank sheet, False: Exit Sub
        End If
        If ActiveSheet.Name <> bank And oldLast >= first Then
            ' An inactive sheet has its own selection, but Excel exposes it only
            ' on activation. Keep its rows in place until its view can be captured.
            mDeferred = True: mDeferredBanks(bank) = True: ProtectBank sheet, False: Exit Sub
        End If
    End If
    phase = "capture view": Set views = CaptureView(sheet)
    On Error GoTo Failed
    sheet.Unprotect PASSWORD
    ' Apply each prototype to whole contiguous new-row ranges. A full initial
    ' load needs one style assignment per column, rather than one per cell.
    For row = 1 To UBound(desired, 1) + 1
        needsFormat = False
        If row <= UBound(desired, 1) Then
            If changed.Exists(CStr(row)) Then needsFormat = (Len(Txt(current(row, PhysicalColumn(bank, 52) - 14))) = 0 Or first + row - 1 = first + order.Count)
        End If
        If needsFormat Then
            If formatStart = 0 Then formatStart = first + row - 1
            formatEnd = first + row - 1
        ElseIf formatStart > 0 Then
            For col = 15 To lastCol
                sheet.Range(sheet.Cells(formatStart, col), sheet.Cells(formatEnd, col)).Style = StylePrefix(bank) & col
            Next
            formatStart = 0
        End If
    Next
    For Each changedRow In changed.Keys
        phase = "write row " & CStr(changedRow)
        row = CLng(changedRow): actualRow = first + row - 1
        wasHidden = CBool(sheet.Rows(actualRow).Hidden)
        col = 1
        Do While col <= UBound(desired, 2)
            If Not differences(row, col) Then
                col = col + 1
            Else
                startCol = col: endCol = col
                Do While endCol < UBound(desired, 2)
                    If Not differences(row, endCol + 1) Then Exit Do
                    endCol = endCol + 1
                Loop
                ReDim block(1 To 1, 1 To endCol - startCol + 1)
                For i = startCol To endCol: block(1, i - startCol + 1) = desired(row, i): Next
                sheet.Range(sheet.Cells(actualRow, startCol + 14), sheet.Cells(actualRow, endCol + 14)).Value2 = block
                col = endCol + 1
            End If
        Loop
        sheet.Rows(actualRow).RowHeight = 12
        If wasHidden Then sheet.Rows(actualRow).Hidden = True
    Next
    If Not mEditableState Is Nothing Then
        If mEditableState.Exists(bank) Then mEditableState.Remove bank
    End If
    If mDeferredBanks.Exists(bank) Then mDeferredBanks.Remove bank
    phase = "protect": ProtectBank sheet, ExcelSync.ViewCanRenderEdit()
    sheet.Calculate
    phase = "restore view": RestoreView sheet, views, positions
    Exit Sub
Failed:
    message = bank & " (" & phase & "): " & Err.Description
    On Error Resume Next
    ProtectBank sheet, False
    RestoreView sheet, views, positions
    On Error GoTo 0
    Err.Raise 5, , message
End Sub

Private Sub ProtectBank(ByVal sheet As Worksheet, ByVal editable As Boolean)
    Dim col As Long, last As Long, bank As String, row As Long, key As String
    Dim stateKey As String
    Dim keys As Variant
    bank = sheet.Name: last = sheet.Cells(sheet.Rows.Count, PhysicalColumn(bank, 52)).End(xlUp).row + 1
    If last < FirstRow(bank) Then last = FirstRow(bank)
    stateKey = CStr(editable) & ":" & CStr(last)
    If mEditableState Is Nothing Then Set mEditableState = Dict()
    If mEditableState.Exists(bank) Then
        If mEditableState(bank) = stateKey And sheet.ProtectContents And sheet.ProtectionMode Then Exit Sub
    End If
    sheet.Unprotect PASSWORD
    sheet.Cells.Locked = True
    If editable Then
        For col = 15 To PhysicalColumn(bank, 53)
            If InputColumn(bank, col) > 0 Then sheet.Range(sheet.Cells(FirstRow(bank), col), sheet.Cells(last, col)).Locked = False
        Next
        If last > FirstRow(bank) Then keys = ReadKeys(sheet, FirstRow(bank), last - 1)
        For row = FirstRow(bank) To last - 1
            key = ExcelSync.ViewDisplayKey(ArrayKey(keys, row - FirstRow(bank) + 1))
            If ExcelSync.ViewIsDeleting(key) Then sheet.Range(sheet.Cells(row, 15), sheet.Cells(row, PhysicalColumn(bank, 53))).Locked = True
        Next
    End If
    sheet.Protect Password:=PASSWORD, UserInterfaceOnly:=True, AllowFiltering:=True
    mEditableState(bank) = stateKey
End Sub

Public Sub SetEditable(ByVal editable As Boolean)
    Dim bank As Variant, events As Boolean, wasSaved As Boolean, message As String
    If Not Enabled() Then Exit Sub
    events = Application.EnableEvents: wasSaved = ThisWorkbook.Saved
    On Error GoTo Failed
    Application.EnableEvents = False
    editable = editable And ExcelSync.ViewCanRenderEdit()
    For Each bank In ViewNames()
        If Not mDeferredBanks Is Nothing Then
            If mDeferredBanks.Exists(CStr(bank)) Then
                ProtectBank ThisWorkbook.Worksheets(CStr(bank)), False
            Else
                ProtectBank ThisWorkbook.Worksheets(CStr(bank)), editable
            End If
        Else
            ProtectBank ThisWorkbook.Worksheets(CStr(bank)), editable
        End If
    Next
Finished:
    Application.EnableEvents = events
    If wasSaved Then ThisWorkbook.Saved = True
    If Len(message) > 0 Then Err.Raise 5, , message
    Exit Sub
Failed:
    message = Err.Description
    GoTo Finished
End Sub

Public Sub UpdateStatus(ByVal text As String)
    Dim bank As Variant, sheet As Worksheet, cell As Range, events As Boolean, saved As Boolean
    If Not Enabled() Then Exit Sub
    events = Application.EnableEvents: saved = ThisWorkbook.Saved
    On Error GoTo Finished
    Application.EnableEvents = False
    For Each bank In ViewNames()
        Set sheet = ThisWorkbook.Worksheets(CStr(bank)): Set cell = sheet.Cells(1, PhysicalColumn(CStr(bank), 50))
        If Txt(cell.Value2) <> text Then cell.Value2 = text
    Next
    SetEditable ExcelSync.ViewCanRenderEdit()
Finished:
    Application.EnableEvents = events
    If saved Then ThisWorkbook.Saved = True
End Sub

Public Sub CellsEdited(ByVal target As Range)
    Dim sheet As Worksheet, bank As String, core As Worksheet, cell As Range, changed As Range, part As Range, col As Long, row As Long, key As String
    Dim events As Boolean, value As Variant, message As String, last As Long, inputRow As Long, values As Object, address As Variant, item As Object, area As Range, bridging As Boolean
    Dim coreKeys As Variant, coreRows As Object, index As Long
    If mRendering Or Not ExcelSync.ViewCanEdit() Then Exit Sub
    Set sheet = target.Worksheet: bank = sheet.Name
    If Not IsBankSheet(bank) Then Exit Sub
    Set core = ExcelSync.ViewCoreSheet(): Set values = Dict()
    last = sheet.Cells(sheet.Rows.Count, PhysicalColumn(bank, 52)).End(xlUp).row + 1
    If last < FirstRow(bank) Then last = FirstRow(bank)
    inputRow = core.Cells(core.Rows.Count, 52).End(xlUp).row + 1
    If inputRow < 5 Then inputRow = 5
    events = Application.EnableEvents
    On Error GoTo Failed
    ReadArticles
    Set area = Intersect(target, sheet.Range(sheet.Cells(FirstRow(bank), 15), sheet.Cells(last, PhysicalColumn(bank, 53))))
    If area Is Nothing Then Exit Sub
    Set coreRows = Dict()
    If inputRow > 5 Then
        coreKeys = core.Range("AY5:AZ" & (inputRow - 1)).Value2
        For index = 1 To UBound(coreKeys, 1)
            key = ArrayKey(coreKeys, index)
            If Len(key) > 0 Then coreRows(key) = index + 4
        Next
    End If
    For Each cell In area.Cells
        col = InputColumn(bank, cell.Column)
        If col > 0 And cell.row >= FirstRow(bank) And cell.row <= last Then
            If cell.HasFormula Or IsError(cell.Value2) Then Err.Raise 5, , "Enter a value, not a formula, in editable cells."
            key = KeyAt(sheet, cell.row)
            If Len(key) = 0 Then
                If cell.row <> last Then Err.Raise 5, , "Use the empty input row below the receipts."
                row = inputRow
            Else
                If Not ExcelSync.ViewRecords().Exists(key) Then Err.Raise 5, , "Wait for the updated receipt before editing."
                If Not coreRows.Exists(key) Then Err.Raise 5, , "Wait for the updated receipt before editing."
                row = CLng(coreRows(key))
            End If
            value = cell.Value2
            If col = 40 Then value = ArticleInput(value, core.Cells(row, 40).Value2)
            Set item = Dict(): item.Add "row", row: item.Add "column", col: item.Add "value", value
            item.Add "old", core.Cells(row, col).Value2
            values.Add cell.Address, item
        End If
    Next
    If values.Count = 0 Then Exit Sub
    Application.EnableEvents = False
    bridging = True
    For Each address In values.Keys
        Set item = values(address): Set part = core.Cells(CLng(item("row")), CLng(item("column")))
        part.Value2 = item("value")
        If changed Is Nothing Then Set changed = part Else Set changed = Union(changed, part)
    Next
    Application.EnableEvents = events
    If bank = AllName() Then
        ExcelSync.CellsEdited changed
    Else
        ExcelSync.CellsEdited changed, bank
    End If
    bridging = False
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    If bridging Then
        Application.EnableEvents = False
        For Each address In values.Keys
            Set item = values(address)
            core.Cells(CLng(item("row")), CLng(item("column"))).Value2 = item("old")
        Next
    End If
    Application.EnableEvents = events
    Render
    On Error GoTo 0
    MsgBox message, vbExclamation
End Sub

Public Sub SelectionChanged(ByVal target As Range)
    If mRendering Or Not IsBankSheet(target.Worksheet.Name) Then Exit Sub
    ExcelSync.ViewSelectKey KeyAt(target.Worksheet, target.row)
End Sub

Public Sub DoubleClick(ByVal target As Range, ByRef cancel As Boolean)
    Dim bank As String
    bank = target.Worksheet.Name
    If Not IsBankSheet(bank) Then Exit Sub
    If target.Column = PhysicalColumn(bank, 50) And target.row >= FirstRow(bank) Then
        cancel = True: ExcelSync.ViewRowAction KeyAt(target.Worksheet, target.row)
    End If
End Sub

Public Sub SelectInputRow()
    Dim sheet As Worksheet, row As Long
    If Not Enabled() Then Exit Sub
    Set sheet = ThisWorkbook.Worksheets("CardXabar")
    If ActiveWorkbook Is ThisWorkbook Then
        If IsBankSheet(ActiveSheet.Name) Then Set sheet = ActiveSheet
    End If
    row = sheet.Cells(sheet.Rows.Count, PhysicalColumn(sheet.Name, 52)).End(xlUp).row + 1
    If row < FirstRow(sheet.Name) Then row = FirstRow(sheet.Name)
    sheet.Activate: sheet.Cells(row, 21).Select
End Sub

Public Sub SortByDate()
    Dim sheet As Worksheet, last As Long, row As Long, key As String, views As Collection, positions As Object
    Dim events As Boolean, screen As Boolean, message As String
    If Not Enabled() Or Not ExcelSync.ViewCanEdit() Then Exit Sub
    If Not ActiveWorkbook Is ThisWorkbook Then Exit Sub
    If Not IsBankSheet(ActiveSheet.Name) Or Application.CutCopyMode <> False Then Exit Sub
    Set sheet = ActiveSheet
    last = sheet.Cells(sheet.Rows.Count, PhysicalColumn(sheet.Name, 52)).End(xlUp).row
    If last < FirstRow(sheet.Name) Then Exit Sub
    events = Application.EnableEvents: screen = Application.ScreenUpdating
    Set views = CaptureView(sheet): Set positions = Dict()
    On Error GoTo Failed
    Application.EnableEvents = False: Application.ScreenUpdating = False
    sheet.Unprotect PASSWORD
    sheet.Range(sheet.Cells(FirstRow(sheet.Name), 15), sheet.Cells(last, PhysicalColumn(sheet.Name, 53))).Sort _
        Key1:=sheet.Cells(FirstRow(sheet.Name), 17), Order1:=xlAscending, Header:=xlNo
    For row = FirstRow(sheet.Name) To last
        key = KeyAt(sheet, row): If Len(key) > 0 Then positions(key) = row
    Next
    If Not mEditableState Is Nothing Then
        If mEditableState.Exists(sheet.Name) Then mEditableState.Remove sheet.Name
    End If
    ProtectBank sheet, ExcelSync.ViewCanRenderEdit()
    RestoreView sheet, views, positions
    Render
Finished:
    Application.EnableEvents = events: Application.ScreenUpdating = screen
    If Len(message) > 0 Then Err.Raise 5, , message
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    ProtectBank sheet, False
    RestoreView sheet, views, positions
    On Error GoTo 0
    GoTo Finished
End Sub

Public Function SelfTest() As String
    Dim fields As Object, desired(1 To 1, 1 To 46) As Variant
    If Same(Empty, 0) Or Same(0, Empty) Or Same(Null, 0) Then Err.Raise 5, , "Blank bank cell matched numeric zero"
    If Not Same(0, 0) Or Not Same(Empty, "") Then Err.Raise 5, , "Bank scalar comparison"
    If PhysicalColumn("HUMO Card", 26) <> 32 Or PhysicalColumn("HUMO Card", 33) <> 40 Then Err.Raise 5, , "Bank amount mapping"
    If InputColumn("CardXabar", 36) <> 54 Or InputColumn("HUMO Card", 43) <> 54 Then Err.Raise 5, , "Bank confirmation mapping"
    If GroupMoney("1234567.89", "CardXabar") <> "1 234 567.89" Then Err.Raise 5, , "Card money display"
    If GroupMoney("1234567.89", "HUMO Card") <> "1.234.567,89" Then Err.Raise 5, , "HUMO money display"
    Set fields = Dict(): fields.Add "amount", "1234.56": fields.Add "transaction_type", "DEBIT": fields.Add "currency", "UZS"
    If MoneyText(fields, "HUMO Card", False) <> U("\u2796") & " 1.234,56 UZS" Then Err.Raise 5, , "Bank debit marker"
    fields.Add "balance_after", "-12.30"
    If MoneyText(fields, "CardXabar", True) <> U("\ud83d\udcb5") & " -12.30 UZS" Then Err.Raise 5, , "Negative bank balance"
    fields.Add "raw_message", "headline" & vbLf & U("\u2796") & " 99.00 USD"
    If MoneyText(fields, "CardXabar", False) <> U("\u2796") & " 99.00 USD" Then Err.Raise 5, , "Original bank money line"
    DigitGroups desired, 1, 26, "1234567.89"
    If desired(1, 12) <> " 1.234.567" Or desired(1, 13) <> 10 Then Err.Raise 5, , "HUMO digit groups"
    If desired(1, 17) <> "567" Or desired(1, 16) <> "234" Or desired(1, 15) <> 1 Then Err.Raise 5, , "HUMO digit order"
    SelfTest = "PASS: bank mappings, raw money, decimal precision, bank signs and digit groups"
End Function
