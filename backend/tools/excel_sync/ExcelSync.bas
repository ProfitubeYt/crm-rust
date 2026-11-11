Attribute VB_Name = "ExcelSync"
Option Explicit

Private Const FIRST_ROW As Long = 5
Private Const ID_COL As Long = 51       ' AY
Private Const UUID_COL As Long = 52     ' AZ
Private Const VERSION_COL As Long = 53  ' BA
Private Const LAYOUT_PASSWORD As String = "tbs-layout"
Private Const IDENTITY_ERROR As Long = vbObjectError + 2101
Private Const REVOKED_ERROR As Long = vbObjectError + 2102
Private Const PROMPT_CANCELLED As Long = 1223
Private mState As Object
' A shared workbook shows receipts only on a computer activated with the
' owner's password. The on-disk file always keeps them hidden.
Private mActivated As Boolean
Private mDeviceId As String
Private mBindTried As Boolean
Private mPrompting As Boolean
Private mPromptDue As Date
Private mPromptProcedure As String
Private mActivationCode As Long
Private mReady As Boolean
Private mBusy As Boolean
Private mWriting As Boolean
Private mDue As Date
Private mScheduledProcedure As String
Private mLockFile As Integer
Private mDirectory As String
Private mWorkbookId As String
Private mSelectedKey As String
Private mLastSaveSuccess As Boolean
Private mHasSnapshot As Boolean
Private mInputsReady As Boolean
Private mGridStylesReady As Boolean
Private mRowAliases As Object
Private mRenderDeferred As Boolean
Private mSnapshotCache As Object
Private mSnapshotTag As String
Private mSnapshotUrl As String
Private mSnapshotUnchanged As Boolean
Private mSnapshotMerged As Boolean

Private Function IsActiveViewWindow(ByVal candidate As Window) As Boolean
    Dim currentWindow As Window
    On Error GoTo MissingWindow
    Set currentWindow = Application.ActiveWindow
    If currentWindow Is Nothing Then Exit Function
    ' Excel exposes different COM proxies for the same window through
    ' ActiveWindow and Workbook.Windows; object identity is not reliable.
    IsActiveViewWindow = (currentWindow.Hwnd = candidate.Hwnd)
MissingWindow:
End Function

Private Function BeginViewUpdate() As Object
    Dim saved As Object, views As New Collection, view As Object, paneViews As Collection, part As Object
    Dim bookWindow As Window, paneView As Pane, selected As Range, active As Range
    Dim windowIndex As Long, paneIndex As Long, captureStep As String, message As String
    Dim areaViews As Collection, areaView As Object, areaKeys As Collection, area As Range
    Dim areaIndex As Long, row As Long, lastDataRow As Long
    On Error GoTo Failed
    captureStep = "application flags"
    Set saved = Dict()
    saved.Add "events", Application.EnableEvents: saved.Add "screen", Application.ScreenUpdating
    saved.Add "writing", mWriting
    captureStep = "Windows.Count"
    For windowIndex = 1 To ThisWorkbook.Windows.Count
        captureStep = "Windows.Item"
        Set bookWindow = ThisWorkbook.Windows.Item(windowIndex)
        captureStep = "Window.ActiveSheet"
        If TypeName(bookWindow.ActiveSheet) = "Worksheet" Then
            If bookWindow.ActiveSheet.Name = DataSheetName() Then
                Set view = Dict(): Set paneViews = New Collection
                view.Add "window", bookWindow
                captureStep = "Window.RangeSelection"
                Set selected = bookWindow.RangeSelection
                captureStep = "Window.ActiveCell"
                Set active = bookWindow.ActiveCell
                view.Add "selection", selected.Address: view.Add "active", active.Address
                view.Add "active_row", active.row
                view.Add "active_key", RowKey(active.row)
                Set areaViews = New Collection
                lastDataRow = Grid.Cells(Grid.Rows.Count, UUID_COL).End(xlUp).row
                For areaIndex = 1 To selected.Areas.Count
                    Set area = selected.Areas.Item(areaIndex)
                    Set areaView = Dict(): Set areaKeys = New Collection
                    areaView.Add "address", area.Address
                    ' Whole-column/header selections retain their exact shape.
                    ' Bounded receipt areas follow their own surviving UUIDs.
                    If area.row >= FIRST_ROW And area.row + area.Rows.Count - 1 <= lastDataRow Then
                        For row = area.row To area.row + area.Rows.Count - 1
                            areaKeys.Add RowKey(row)
                        Next
                    End If
                    areaView.Add "keys", areaKeys: areaViews.Add areaView
                Next
                view.Add "areas", areaViews
                captureStep = "Panes.Count"
                For paneIndex = 1 To bookWindow.Panes.Count
                    captureStep = "Panes.Item(" & paneIndex & ")"
                    Set paneView = bookWindow.Panes.Item(paneIndex)
                    Set part = Dict(): part.Add "pane", paneView
                    captureStep = "Pane.ScrollRow/ScrollColumn"
                    part.Add "row", paneView.ScrollRow: part.Add "column", paneView.ScrollColumn
                    part.Add "key", RowKey(paneView.ScrollRow)
                    paneViews.Add part
                Next
                view.Add "panes", paneViews: views.Add view
            End If
        End If
    Next
    saved.Add "windows", views
    captureStep = "disable screen/events"
    mWriting = True: Application.EnableEvents = False: Application.ScreenUpdating = False
    Set BeginViewUpdate = saved
    Exit Function
Failed:
    message = Err.Description
    On Error Resume Next
    If Not saved Is Nothing Then
        If saved.Exists("writing") Then mWriting = CBool(saved("writing"))
        If saved.Exists("events") Then Application.EnableEvents = CBool(saved("events"))
        If saved.Exists("screen") Then Application.ScreenUpdating = CBool(saved("screen"))
    End If
    On Error GoTo 0
    Err.Raise 5, , "Cannot preserve Excel view (" & captureStep & "): " & message
End Function

Private Sub EndViewUpdate(ByVal saved As Object, Optional ByVal rowPositions As Object = Nothing)
    Dim view As Variant, part As Variant, win As Window, paneView As Pane, targetRow As Long, key As String
    Dim selected As Range, active As Range, area As Range, moved As Range, item As Range
    Dim areaView As Variant, recordKey As Variant, firstRow As Long, lastRow As Long, selectionChanged As Boolean
    If saved Is Nothing Then Exit Sub
    ' Restoring a background window must never activate it or select its cells.
    ' Ordinary refreshes do not Select at all. Selection moves only when a
    ' deletion above it moved the selected receipt to another row.
    On Error Resume Next
    For Each view In saved("windows")
        Set win = view("window")
        If IsActiveViewWindow(win) Then
            If Not rowPositions Is Nothing Then
                Set moved = Nothing: selectionChanged = False
                For Each areaView In view("areas")
                    Set area = Grid.Range(CStr(areaView("address")))
                    firstRow = 0: lastRow = 0
                    For Each recordKey In areaView("keys")
                        If rowPositions.Exists(CStr(recordKey)) Then
                            targetRow = CLng(rowPositions(CStr(recordKey)))
                            If firstRow = 0 Or targetRow < firstRow Then firstRow = targetRow
                            If targetRow > lastRow Then lastRow = targetRow
                        End If
                    Next
                    Set item = area
                    If firstRow > 0 Then Set item = Grid.Range(Grid.Cells(firstRow, area.Column), Grid.Cells(lastRow, area.Column + area.Columns.Count - 1))
                    If item.Address <> area.Address Then selectionChanged = True
                    If moved Is Nothing Then Set moved = item Else Set moved = Union(moved, item)
                Next
                If selectionChanged Then moved.Select
                Set active = Grid.Range(CStr(view("active")))
                key = CStr(view("active_key"))
                If rowPositions.Exists(key) Then Set active = Grid.Cells(CLng(rowPositions(key)), active.Column)
                If active.Address <> win.ActiveCell.Address Then active.Activate
            End If
        End If
        For Each part In view("panes")
            Set paneView = part("pane"): targetRow = CLng(part("row"))
            If Not rowPositions Is Nothing Then
                key = CStr(part("key"))
                If rowPositions.Exists(key) Then targetRow = CLng(rowPositions(key))
            End If
            If paneView.ScrollRow <> targetRow Then paneView.ScrollRow = targetRow
            If paneView.ScrollColumn <> CLng(part("column")) Then paneView.ScrollColumn = CLng(part("column"))
        Next
    Next
    mWriting = CBool(saved("writing"))
    Application.EnableEvents = CBool(saved("events"))
    Application.ScreenUpdating = CBool(saved("screen"))
    On Error GoTo 0
End Sub

Public Function DataSheetName() As String
    DataSheetName = CStr(ThisWorkbook.Worksheets("_sync").Range("B3").Value2)
End Function

Private Function Grid() As Worksheet
    Set Grid = ThisWorkbook.Worksheets(DataSheetName())
End Function

Private Function Records() As Object
    Set Records = mState("records")
End Function

Private Function Intents() As Object
    Set Intents = mState("intents")
End Function

Private Function Outbox() As Object
    Set Outbox = mState("outbox")
End Function

Private Function Blocked() As Object
    Set Blocked = mState("blocked")
End Function

Public Function ViewRecords() As Object
    If mState Is Nothing Then Exit Function
    Set ViewRecords = Records()
End Function

Public Function ViewFields(ByVal key As String) As Object
    Dim fields As Object
    Set fields = Records.Item(key)("fields")
    ' Adapter contract is read-only. Avoid cloning every full receipt on each
    ' render; only old schemas or pending edits need a separate effective view.
    If Not Intents.Exists(key) And fields.Exists("excel_bank") And fields.Exists("p2p_confirmed") Then
        Set ViewFields = fields
    Else
        Set ViewFields = EffectiveFields(key)
    End If
End Function

Public Function ViewCoreSheet() As Worksheet
    Set ViewCoreSheet = Grid()
End Function

Public Function ViewCoreRow(ByVal key As String) As Long
    If Not mReady Then Exit Function
    If Not Records.Exists(key) Then Exit Function
    ViewCoreRow = FindRow(key)
End Function

Public Function ViewDisplayKey(ByVal key As String) As String
    ViewDisplayKey = DisplayKey(key)
End Function

Public Sub ViewSelectKey(ByVal key As String)
    If Not mReady Or mWriting Then Exit Sub
    mSelectedKey = DisplayKey(key)
End Sub

Public Sub ViewRowAction(ByVal key As String)
    If Not mReady Or mBusy Or mWriting Then Exit Sub
    key = DisplayKey(key)
    If Not Records.Exists(key) Then Exit Sub
    mSelectedKey = key
    If Blocked.Exists(key) Then ResolveSelected Else DeleteSelected
End Sub

Public Function ViewCanEdit() As Boolean
    ViewCanEdit = ViewCanRenderEdit() And Not mWriting And Not mBusy
End Function

Public Function ViewCanRenderEdit() As Boolean
    If Not mReady Or Not mActivated Or Not mHasSnapshot Or mRenderDeferred Then Exit Function
    If CBool(mState("paused")) Then Exit Function
    ViewCanRenderEdit = True
End Function

Public Function ViewIsDeleting(ByVal key As String) As Boolean
    If Not mReady Then Exit Function
    If Intents.Exists(key) Then ViewIsDeleting = (Intents.Item(key)("kind") = "delete")
End Function

Private Function KeyOf(ByVal record As Object) As String
    If record.Exists("_local_key") Then KeyOf = CStr(record("_local_key")): Exit Function
    If Len(TextValue(record("uuid"))) > 0 Then
        KeyOf = "u:" & CStr(record("uuid"))
    Else
        KeyOf = "i:" & CStr(record("id"))
    End If
End Function

Private Function RowKey(ByVal row As Long) As String
    If Len(CStr(Grid.Cells(row, UUID_COL).Value2)) > 0 Then
        RowKey = "u:" & CStr(Grid.Cells(row, UUID_COL).Value2)
    ElseIf Len(CStr(Grid.Cells(row, ID_COL).Value2)) > 0 Then
        RowKey = "i:" & CStr(Grid.Cells(row, ID_COL).Value2)
    End If
End Function

Private Function DisplayKey(ByVal key As String) As String
    Dim aliases As Object, step As Long, nextKey As String
    DisplayKey = key
    If Not mRowAliases Is Nothing Then
        If mRowAliases.Exists(key) Then DisplayKey = CStr(mRowAliases(key))
    End If
    If mState Is Nothing Then Exit Function
    If Not mState.Exists("view_aliases") Then Exit Function
    Set aliases = mState("view_aliases")
    ' A bank sheet can remain deferred across saves/reopens. Keep its draft
    ' anchors usable after an ACK without touching any immutable operation.
    For step = 1 To aliases.Count
        If Not aliases.Exists(DisplayKey) Then Exit Function
        nextKey = CStr(aliases(DisplayKey))
        If nextKey = DisplayKey Then Exit Function
        DisplayKey = nextKey
    Next
End Function

Private Function Pending(ByVal key As String) As Boolean
    Dim opId As Variant
    If Intents.Exists(key) Then Pending = True: Exit Function
    For Each opId In Outbox.Keys
        If KeyOf(Outbox.Item(opId)) = key Then Pending = True: Exit Function
    Next
End Function

Private Function HasOperation(ByVal key As String) As Boolean
    Dim opId As Variant
    For Each opId In Outbox.Keys
        If KeyOf(Outbox.Item(opId)) = key Then HasOperation = True: Exit Function
    Next
End Function

Private Function HasUnreconciledCreate() As Boolean
    Dim opId As Variant, key As String, rejection As Object
    For Each opId In Outbox.Keys
        If Outbox.Item(opId)("kind") = "create" Then
            key = KeyOf(Outbox.Item(opId))
            If Not Blocked.Exists(key) Then HasUnreconciledCreate = True: Exit Function
            Set rejection = Blocked.Item(key)
            If rejection.Exists("applied") Then
                If rejection("applied") Then HasUnreconciledCreate = True: Exit Function
            End If
            ' A conclusive first rejection has no corresponding server receipt.
            If rejection("status") <> "invalid" And rejection("status") <> "forbidden" Then HasUnreconciledCreate = True: Exit Function
        End If
    Next
End Function

Private Sub Status(ByVal text As String)
    Dim wasWriting As Boolean, hadEvents As Boolean, wasSaved As Boolean, lockSheet As Worksheet
    wasWriting = mWriting: hadEvents = Application.EnableEvents
    wasSaved = ThisWorkbook.Saved
    mWriting = True: Application.EnableEvents = False
    On Error GoTo Finished
    If TextValue(Grid.Range("T1").Value2) <> text Then Grid.Range("T1").Value2 = text
    If TextValue(ThisWorkbook.Worksheets("_sync").Range("B9").Value2) <> text Then ThisWorkbook.Worksheets("_sync").Range("B9").Value2 = text
    BankViews.UpdateStatus text
    Set lockSheet = FindLockSheet()
    If Not lockSheet Is Nothing Then
        If TextValue(lockSheet.Range("A6").Value2) <> text Then lockSheet.Range("A6").Value2 = text
    End If
Finished:
    Application.EnableEvents = hadEvents: mWriting = wasWriting
    ' A transient heartbeat is not a reason to save the entire workbook.
    ' Never mark an already dirty workbook saved: it may contain user edits.
    If wasSaved Then ThisWorkbook.Saved = True
    ' The status bar is cosmetic and rejects long server/error messages.
    On Error Resume Next
    If ActiveWorkbook Is ThisWorkbook Then Application.StatusBar = Left$("TBSparcer: " & text, 255)
    On Error GoTo 0
End Sub

Public Sub ResumeView()
    If Not mReady Or mBusy Or mWriting Or Not mActivated Then Exit Sub
    If Application.CutCopyMode <> False Then Schedule 5: Exit Sub
    On Error GoTo Failed
    ShowViews True
    If mRenderDeferred Then
        RenderGrid
    ElseIf mHasSnapshot Then
        BankViews.Render
    End If
    BankViews.SetEditable ViewCanEdit()
    SaveView
    If Intents.Count > 0 Then Schedule 5 Else Schedule 60
    Exit Sub
Failed:
    Status Err.Description
    Schedule 60
End Sub

Private Function Ui(ByVal escaped As String) As String
    ' ASCII VBA sources; JSON unescapes localized labels into UTF-16.
    Dim value As Object
    Static labels As Object
    If labels Is Nothing Then Set labels = Dict()
    If labels.Exists(escaped) Then Ui = labels(escaped): Exit Function
    Set value = JsonParse("{""text"":""" & escaped & """}")
    Ui = CStr(value("text"))
    labels.Add escaped, Ui
End Function

Private Function SharedProfile(ByVal cfg As Worksheet) As Boolean
    SharedProfile = TextValue(cfg.Range("B8").Value2) = "1" And Len(Trim$(TextValue(cfg.Range("B5").Value2))) > 0 _
        And Len(Trim$(TextValue(cfg.Range("B7").Value2))) > 0
End Function

Private Function Gated() As Boolean
    On Error Resume Next
    Gated = SharedProfile(ThisWorkbook.Worksheets("_sync"))
End Function

Public Function LockSheetName() As String
    LockSheetName = Ui("\u0414\u043e\u0441\u0442\u0443\u043f")
End Function

Public Function IsActivated() As Boolean
    IsActivated = mActivated
End Function

Private Function FindLockSheet() As Worksheet
    On Error Resume Next
    Set FindLockSheet = ThisWorkbook.Worksheets(LockSheetName())
End Function

Private Function ContentSheets() As Variant
    If BankViews.Enabled() Then ContentSheets = BankViews.Views() Else ContentSheets = Array(DataSheetName())
End Function

Private Function IsShown(ByVal names As Variant) As Boolean
    Dim name As Variant, lockSheet As Worksheet
    For Each name In names
        If ThisWorkbook.Worksheets(CStr(name)).Visible <> xlSheetVisible Then Exit Function
    Next
    Set lockSheet = FindLockSheet()
    If Not lockSheet Is Nothing Then
        If lockSheet.Visible = xlSheetVisible Then Exit Function
    End If
    IsShown = True
End Function

Private Function IsHidden(ByVal names As Variant) As Boolean
    Dim name As Variant, lockSheet As Worksheet
    Set lockSheet = FindLockSheet()
    If lockSheet Is Nothing Then Exit Function
    If lockSheet.Visible <> xlSheetVisible Then Exit Function
    For Each name In names
        If ThisWorkbook.Worksheets(CStr(name)).Visible <> xlSheetVeryHidden Then Exit Function
    Next
    IsHidden = True
End Function

Private Sub WriteLockText(ByVal sheet As Worksheet)
    sheet.Columns("A").ColumnWidth = 110
    sheet.Range("A1").Value2 = Ui("\u0427\u0435\u043a\u0438 \u0437\u0430\u0449\u0438\u0449\u0435\u043d\u044b")
    sheet.Range("A1").Font.Bold = True: sheet.Range("A1").Font.Size = 16
    sheet.Range("A3").Value2 = Ui("\u0427\u0435\u043a\u0438 \u043e\u0442\u043a\u0440\u044b\u0432\u0430\u044e\u0442\u0441\u044f \u0442\u043e\u043b\u044c\u043a\u043e \u043d\u0430 \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440\u0435, \u0430\u043a\u0442\u0438\u0432\u0438\u0440\u043e\u0432\u0430\u043d\u043d\u043e\u043c \u043f\u0430\u0440\u043e\u043b\u0435\u043c \u0432\u043b\u0430\u0434\u0435\u043b\u044c\u0446\u0430.")
    sheet.Range("A4").Value2 = Ui("\u0412\u043a\u043b\u044e\u0447\u0438\u0442\u0435 \u043c\u0430\u043a\u0440\u043e\u0441\u044b \u0438 \u0432\u0432\u0435\u0434\u0438\u0442\u0435 \u043f\u0430\u0440\u043e\u043b\u044c. \u0427\u0442\u043e\u0431\u044b \u0432\u0432\u0435\u0441\u0442\u0438 \u0435\u0433\u043e \u0441\u043d\u043e\u0432\u0430, \u0434\u0432\u0430\u0436\u0434\u044b \u0449\u0451\u043b\u043a\u043d\u0438\u0442\u0435 \u043f\u043e \u044d\u0442\u043e\u043c\u0443 \u043b\u0438\u0441\u0442\u0443.")
    sheet.Range("A6").NumberFormat = "@"
End Sub

Private Function CreateLockSheet() As Worksheet
    Dim sheet As Worksheet
    Set sheet = ThisWorkbook.Worksheets.Add(Before:=ThisWorkbook.Sheets(1))
    sheet.Name = LockSheetName()
    WriteLockText sheet
    sheet.Cells.Locked = True
    sheet.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True
    Set CreateLockSheet = sheet
End Function

Private Sub SetVisible(ByVal sheet As Worksheet, ByVal state As XlSheetVisibility)
    If sheet.Visible <> state Then sheet.Visible = state
End Sub

Private Sub RememberView(ByVal names As Variant)
    Dim name As Variant, cfg As Worksheet
    On Error Resume Next
    For Each name In names
        If ThisWorkbook.ActiveSheet.Name = CStr(name) Then
            Set cfg = ThisWorkbook.Worksheets("_sync")
            If TextValue(cfg.Range("B10").Value2) <> CStr(name) Then
                cfg.Range("B10").NumberFormat = "@": cfg.Range("B10").Value2 = CStr(name)
            End If
            Exit Sub
        End If
    Next
End Sub

Private Function RevealTarget(ByVal names As Variant) As String
    Dim name As Variant, remembered As String
    On Error Resume Next
    remembered = TextValue(ThisWorkbook.Worksheets("_sync").Range("B10").Value2)
    On Error GoTo 0
    RevealTarget = CStr(names(LBound(names)))
    For Each name In names
        If CStr(name) = remembered Then RevealTarget = remembered
    Next
End Function

' Lock: only the lock sheet stays visible. Reveal: the views come back and the
' lock sheet disappears. Excel activates the last sheet unhidden in a
' background book, so the target view is unhidden last; a foreground book is
' activated explicitly (activating a background book would steal focus).
Private Sub ShowViews(ByVal reveal As Boolean)
    Dim names As Variant, name As Variant, lockSheet As Worksheet, target As String, message As String
    Dim hadEvents As Boolean, hadScreen As Boolean, wasWriting As Boolean, wasSaved As Boolean, structure As Boolean
    names = ContentSheets()
    If reveal Then
        If IsShown(names) Then Exit Sub
    ElseIf IsHidden(names) Then
        Exit Sub
    End If
    hadEvents = Application.EnableEvents: hadScreen = Application.ScreenUpdating
    wasWriting = mWriting: wasSaved = ThisWorkbook.Saved
    On Error GoTo Failed
    mWriting = True: Application.EnableEvents = False: Application.ScreenUpdating = False
    structure = ThisWorkbook.ProtectStructure
    If structure Then ThisWorkbook.Unprotect LAYOUT_PASSWORD
    If reveal Then
        target = RevealTarget(names)
        For Each name In names
            If CStr(name) <> target Then SetVisible ThisWorkbook.Worksheets(CStr(name)), xlSheetVisible
        Next
        SetVisible ThisWorkbook.Worksheets(target), xlSheetVisible
        If ActiveWorkbook Is ThisWorkbook Then ThisWorkbook.Worksheets(target).Activate
        Set lockSheet = FindLockSheet()
        If Not lockSheet Is Nothing Then SetVisible lockSheet, xlSheetVeryHidden
    Else
        Set lockSheet = FindLockSheet()
        If lockSheet Is Nothing Then Set lockSheet = CreateLockSheet()
        RememberView names
        SetVisible lockSheet, xlSheetVisible
        For Each name In names
            SetVisible ThisWorkbook.Worksheets(CStr(name)), xlSheetVeryHidden
        Next
    End If
Finished:
    On Error Resume Next
    If structure And Not ThisWorkbook.ProtectStructure Then ThisWorkbook.Protect Password:=LAYOUT_PASSWORD, Structure:=True
    Application.ScreenUpdating = hadScreen
    Application.EnableEvents = hadEvents: mWriting = wasWriting
    If wasSaved Then ThisWorkbook.Saved = True
    On Error GoTo 0
    If Len(message) > 0 Then Err.Raise 5, , message
    Exit Sub
Failed:
    If reveal Then message = "Cannot show the receipts: " Else message = "Cannot hide the receipts: "
    message = message & Err.Description
    Resume Finished
End Sub

Private Sub RequireActivation(Optional ByVal prompt As Boolean = True, Optional ByVal reason As String = "")
    Status reason & Ui("\u0427\u0435\u043a\u0438 \u0441\u043a\u0440\u044b\u0442\u044b: \u044d\u0442\u043e\u0442 \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440 \u043d\u0435 \u0430\u043a\u0442\u0438\u0432\u0438\u0440\u043e\u0432\u0430\u043d. \u0414\u0432\u0430\u0436\u0434\u044b \u0449\u0451\u043b\u043a\u043d\u0438\u0442\u0435 \u043b\u0438\u0441\u0442 \u00ab\u0414\u043e\u0441\u0442\u0443\u043f\u00bb \u0438 \u0432\u0432\u0435\u0434\u0438\u0442\u0435 \u043f\u0430\u0440\u043e\u043b\u044c \u0432\u043b\u0430\u0434\u0435\u043b\u044c\u0446\u0430.")
    If Not prompt Or mPromptDue <> 0 Or mPrompting Then Exit Sub
    ' Only a person gets a dialog; automation (and tests) must never block.
    If Not (Application.UserControl And Application.Visible And Application.Interactive) Then Exit Sub
    On Error Resume Next
    mPromptDue = Now + TimeSerial(0, 0, 1)
    mPromptProcedure = "'" & Replace$(ThisWorkbook.Name, "'", "''") & "'!ExcelSync.PromptActivation"
    Application.OnTime mPromptDue, mPromptProcedure
    If Err.Number <> 0 Then mPromptDue = 0
End Sub

Private Sub CancelPrompt()
    If mPromptDue = 0 Then Exit Sub
    On Error Resume Next
    Application.OnTime EarliestTime:=mPromptDue, Procedure:=mPromptProcedure, Schedule:=False
    mPromptDue = 0
    mPromptProcedure = vbNullString
End Sub

Public Sub LockSheetAction()
    If Not mReady Then StartSync
    If Not mReady Then Exit Sub
    If mActivated Then
        On Error Resume Next
        ShowViews True
    Else
        PromptActivation
    End If
End Sub

Public Sub PromptActivation()
    Dim password As String, result As Long, intro As String, message As String, answer As String, typed As Variant
    mPromptDue = 0
    If Not mReady Or mActivated Or mPrompting Then Exit Sub
    mPrompting = True
    On Error GoTo Finished
    intro = Ui("\u0412\u0432\u0435\u0434\u0438\u0442\u0435 \u043f\u0430\u0440\u043e\u043b\u044c, \u043a\u043e\u0442\u043e\u0440\u044b\u0439 \u0432\u043b\u0430\u0434\u0435\u043b\u0435\u0446 \u0437\u0430\u0434\u0430\u043b \u0434\u043b\u044f \u044d\u0442\u043e\u0439 \u043a\u043d\u0438\u0433\u0438 \u0432 \u0432\u0435\u0431-\u043f\u0430\u043d\u0435\u043b\u0438 TBSparcer. \u041f\u043e\u0441\u043b\u0435 \u043f\u0440\u043e\u0432\u0435\u0440\u043a\u0438 \u044d\u0442\u043e\u0442 \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440 \u0431\u0443\u0434\u0435\u0442 \u0437\u0430\u043f\u043e\u043c\u043d\u0435\u043d.")
    message = intro
    Do
        result = PromptPassword(Ui("TBSparcer: \u0434\u043e\u0441\u0442\u0443\u043f \u043a \u0447\u0435\u043a\u0430\u043c"), message, password)
        If result = PROMPT_CANCELLED Then Exit Do
        If result <> 0 Then
            ' No Windows dialog: Excel's own box still keeps Unicode text.
            typed = Application.InputBox(message, "TBSparcer", Type:=2)
            If VarType(typed) = vbBoolean Then Exit Do
            password = CStr(typed): typed = Empty
        End If
        If Len(password) = 0 Then
            message = Ui("\u041f\u0430\u0440\u043e\u043b\u044c \u043d\u0435 \u0432\u0432\u0435\u0434\u0451\u043d. ") & intro
        Else
            answer = ActivateWithPassword(password)
            password = vbNullString
            If Len(answer) = 0 Then Exit Do
            If mActivationCode <> 401 Then
                UnicodeMsgBox answer, vbExclamation, "TBSparcer"
                Exit Do
            End If
            message = answer & " " & intro
        End If
    Loop
Finished:
    password = vbNullString
    mPrompting = False
End Sub

' Checks the owner's password on the server. On success this computer gets
' its own key (Credential Manager) and the receipts are shown; otherwise the
' reason is returned. Also the automation hook for native tests.
Public Function ActivateWithPassword(ByVal password As String) As String
    Dim body As Object, answer As Object, code As Long, token As String, detail As String
    mActivationCode = 0
    If Not mReady Then ActivateWithPassword = "Synchronization is not running": Exit Function
    If mActivated Then Exit Function
    On Error GoTo Failed
    Set body = Dict()
    body.Add "device_name", DeviceLabel(): body.Add "password", password
    password = vbNullString
    Set answer = Exchange("/api/excel/v1/activate", JsonStringify(body), "", code)
    body.RemoveAll
    mActivationCode = code
    detail = ErrorDetail(answer)
    Select Case code
        Case 201
            token = TextValue(answer("token"))
            If Left$(token, 6) <> "excel_" Or Len(token) < 40 Or Len(token) > 200 Then Err.Raise 5, , "The server returned an invalid key"
            SaveCredential mWorkbookId, token
            token = vbNullString
            mState("device_bound") = True: mState("paused") = False
            Persist
            mActivated = True: mBindTried = True
            ClearSnapshotCache
            ShowViews True
            Status Ui("\u041a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440 \u0430\u043a\u0442\u0438\u0432\u0438\u0440\u043e\u0432\u0430\u043d. \u0417\u0430\u0433\u0440\u0443\u0437\u043a\u0430 \u0447\u0435\u043a\u043e\u0432\u2026")
            Schedule 1
        Case 401
            ActivateWithPassword = Ui("\u041d\u0435\u0432\u0435\u0440\u043d\u044b\u0439 \u043f\u0430\u0440\u043e\u043b\u044c.")
        Case 429
            ActivateWithPassword = Ui("\u0421\u043b\u0438\u0448\u043a\u043e\u043c \u043c\u043d\u043e\u0433\u043e \u043d\u0435\u0432\u0435\u0440\u043d\u044b\u0445 \u043f\u043e\u043f\u044b\u0442\u043e\u043a. \u041f\u043e\u0432\u0442\u043e\u0440\u0438\u0442\u0435 \u0447\u0435\u0440\u0435\u0437 15 \u043c\u0438\u043d\u0443\u0442.")
        Case 403
            Select Case detail
                Case "excel_activation_unavailable"
                    ActivateWithPassword = Ui("\u0412\u043b\u0430\u0434\u0435\u043b\u0435\u0446 \u0435\u0449\u0451 \u043d\u0435 \u0437\u0430\u0434\u0430\u043b \u043f\u0430\u0440\u043e\u043b\u044c \u0434\u043b\u044f \u044d\u0442\u043e\u0439 \u043a\u043d\u0438\u0433\u0438 \u0432 \u0432\u0435\u0431-\u043f\u0430\u043d\u0435\u043b\u0438.")
                Case "permissions_changed_reset_excel_password", "scope_changed_reset_excel_password"
                    ActivateWithPassword = Ui("\u041f\u0440\u0430\u0432\u0430 \u0432\u043b\u0430\u0434\u0435\u043b\u044c\u0446\u0430 \u0438\u0437\u043c\u0435\u043d\u0438\u043b\u0438\u0441\u044c. \u0412\u043b\u0430\u0434\u0435\u043b\u0435\u0446 \u0434\u043e\u043b\u0436\u0435\u043d \u0437\u0430\u043d\u043e\u0432\u043e \u0437\u0430\u0434\u0430\u0442\u044c \u043f\u0430\u0440\u043e\u043b\u044c \u043a\u043d\u0438\u0433\u0438 \u0432 \u0432\u0435\u0431-\u043f\u0430\u043d\u0435\u043b\u0438.")
                Case Else
                    ActivateWithPassword = Ui("\u0414\u043e\u0441\u0442\u0443\u043f \u0437\u0430\u043f\u0440\u0435\u0449\u0451\u043d \u0441\u0435\u0440\u0432\u0435\u0440\u043e\u043c: ") & detail
            End Select
        Case Else
            ActivateWithPassword = Ui("\u0421\u0435\u0440\u0432\u0435\u0440 \u0432\u0435\u0440\u043d\u0443\u043b \u043e\u0448\u0438\u0431\u043a\u0443 HTTP ") & code & IIf(Len(detail) > 0, ": " & detail, "")
    End Select
    Exit Function
Failed:
    token = vbNullString
    ActivateWithPassword = Ui("\u041d\u0435 \u0443\u0434\u0430\u043b\u043e\u0441\u044c \u0430\u043a\u0442\u0438\u0432\u0438\u0440\u043e\u0432\u0430\u0442\u044c \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440: ") & Err.Description
End Function

Private Function DeviceBound() As Boolean
    If mState.Exists("device_bound") Then DeviceBound = CBool(mState("device_bound"))
End Function

Private Function DeviceId() As String
    If Len(mDeviceId) = 0 Then mDeviceId = MachineIdentity()
    DeviceId = mDeviceId
End Function

Private Function DeviceIdOrBlank() As String
    On Error Resume Next
    DeviceIdOrBlank = DeviceId()
End Function

' Trades the old shared key for one bound to this computer. The server keeps
' the old key alive for other computers; any failure just retries next open.
Private Function BindDevice(ByVal token As String) As String
    Dim body As Object, answer As Object, code As Long, fresh As String
    BindDevice = token
    On Error GoTo Finished
    Set body = Dict(): body.Add "device_name", DeviceLabel()
    Set answer = Exchange("/api/excel/v1/device", JsonStringify(body), token, code)
    token = vbNullString
    If code = 201 Then
        fresh = TextValue(answer("token"))
        If Left$(fresh, 6) = "excel_" And Len(fresh) >= 40 And Len(fresh) <= 200 Then
            SaveCredential mWorkbookId, fresh
            BindDevice = fresh
            mState("device_bound") = True
            Persist
        End If
    ElseIf code = 409 Then
        mState("device_bound") = True
        Persist
    End If
Finished:
    fresh = vbNullString
End Function

Private Function Exchange(ByVal path As String, ByVal body As String, ByVal token As String, ByRef code As Long) As Object
    Dim http As Object, url As String
    url = TextValue(mState("server_url"))
    If Not ValidServerUrl(url) Then Err.Raise 5, , "Connect to a valid server first"
    Set http = CreateObject("WinHttp.WinHttpRequest.5.1")
    http.SetTimeouts 3000, 3000, 5000, 15000
    http.Option(6) = False ' Never forward credentials through redirects.
    http.Open "POST", url & path, False
    If Len(token) > 0 Then http.SetRequestHeader "Authorization", "Bearer " & token
    token = vbNullString
    http.SetRequestHeader "X-Excel-Workbook", mWorkbookId
    http.SetRequestHeader "X-Excel-Device", DeviceId()
    http.SetRequestHeader "Content-Type", "application/json; charset=utf-8"
    http.Send body
    body = vbNullString
    code = http.Status
    On Error Resume Next
    Set Exchange = JsonParse(DecodeUtf8(http.ResponseBody))
End Function

Private Function ErrorDetail(ByVal answer As Object) As String
    On Error Resume Next
    If answer Is Nothing Then Exit Function
    If Not answer.Exists("detail") Then Exit Function
    If Not IsObject(answer("detail")) Then ErrorDetail = TextValue(answer("detail"))
End Function

Private Function ResponseDetail(ByVal http As Object) As String
    On Error Resume Next
    ResponseDetail = ErrorDetail(JsonParse(DecodeUtf8(http.ResponseBody)))
End Function

Private Sub ConfigureSharedProfile(ByVal cfg As Worksheet)
    Dim url As String, owner As String, priorOwner As String, key As Variant
    ' The connection key no longer travels inside the file: a copy carried to
    ' another computer must ask for the owner's password. Drop an old one.
    If Len(TextValue(cfg.Range("B6").Value2)) > 0 Then cfg.Range("B6").ClearContents
    url = Trim$(TextValue(cfg.Range("B5").Value2))
    owner = Trim$(TextValue(cfg.Range("B7").Value2))
    If Len(url) + Len(owner) = 0 Then Exit Sub
    If TextValue(cfg.Range("B8").Value2) <> "1" Then Err.Raise IDENTITY_ERROR, , "Shared connection is disabled"
    If Not ValidServerUrl(url) Or Len(owner) = 0 Then Err.Raise 5, , "The shared workbook connection is incomplete"
    If Len(TextValue(mState("server_url"))) > 0 And TextValue(mState("server_url")) <> url Then Err.Raise IDENTITY_ERROR, , "Local changes belong to another server"
    If mState.Exists("owner_id") Then
        priorOwner = TextValue(mState("owner_id"))
        If Len(TextValue(mState("owner_id"))) > 0 And TextValue(mState("owner_id")) <> owner Then Err.Raise IDENTITY_ERROR, , "Local changes belong to another owner"
    End If
    If Len(priorOwner) = 0 Then
        If Outbox.Count > 0 Then Err.Raise IDENTITY_ERROR, , "An unbound pending queue cannot be transferred to a shared owner"
        For Each key In Records.Keys
            If Len(TextValue(Records.Item(key)("id"))) > 0 Then Err.Raise IDENTITY_ERROR, , "Unbound cached records cannot be transferred to a shared owner"
        Next
    End If
    ' Every sync authenticates the snapshot owner BEFORE it can POST a mutation.
    ' The key itself comes from this computer's activation (Credential Manager).
    mState("server_url") = url: mState("owner_id") = owner: mState("paused") = False
    Persist
End Sub

Public Sub StartSync()
    Dim fso As Object, pattern As Object, cfg As Worksheet, filename As String, lockSheet As Worksheet
    If mReady Then Exit Sub
    mActivated = False
    ClearSnapshotCache
    On Error GoTo Failed
    Set cfg = ThisWorkbook.Worksheets("_sync")
    If ThisWorkbook.ReadOnly Or Len(ThisWorkbook.Path) = 0 Then Err.Raise 5, , "Open a writable local copy of the workbook"
    cfg.Unprotect LAYOUT_PASSWORD
    cfg.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True
    Set lockSheet = FindLockSheet()
    If Not lockSheet Is Nothing Then
        lockSheet.Unprotect LAYOUT_PASSWORD
        lockSheet.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True
    End If
    mWorkbookId = CStr(cfg.Range("B1").Value2)
    If Len(mWorkbookId) = 0 Then
        If ThisWorkbook.ReadOnly Or Len(ThisWorkbook.Path) = 0 Then Err.Raise 5, , "Save an editable copy before connecting"
        mWorkbookId = NewUuid()
        cfg.Unprotect LAYOUT_PASSWORD
        cfg.Range("B1").NumberFormat = "@": cfg.Range("B1").Value2 = mWorkbookId
        cfg.Protect LAYOUT_PASSWORD
        ThisWorkbook.Save
    End If
    Set pattern = CreateObject("VBScript.RegExp")
    pattern.Pattern = "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
    If Not pattern.Test(mWorkbookId) Then Err.Raise 5, , "Invalid workbook ID"
    mDirectory = StateDirectory(mWorkbookId)
    mLockFile = FreeFile
    Open mDirectory & "\client.lock" For Binary Access Read Write Lock Read Write As #mLockFile
    filename = mDirectory & "\state.json"
    Set fso = CreateObject("Scripting.FileSystemObject")
    If fso.FileExists(filename) Then
        Set mState = JsonParse(ReadUtf8(filename))
        If TextValue(mState("workbook_id")) <> mWorkbookId Then Err.Raise 5, , "Local state belongs to another workbook"
        If mState("schema") <> 1 Then Err.Raise 5, , "Unsupported local state version"
        ' Never replace a corrupt/unrecognised queue with an empty one.
        If Not IsObject(mState("records")) Then Err.Raise 5, , "Invalid local records"
        If Not IsObject(mState("outbox")) Then Err.Raise 5, , "Invalid local queue"
        If Not IsObject(mState("intents")) Then Err.Raise 5, , "Invalid local intentions"
        If Not IsObject(mState("blocked")) Then Err.Raise 5, , "Invalid local conflicts"
    Else
        Set mState = Dict()
        mState.Add "schema", 1: mState.Add "workbook_id", mWorkbookId
        mState.Add "records", Dict(): mState.Add "intents", Dict()
        mState.Add "outbox", Dict(): mState.Add "blocked", Dict()
        mState.Add "server_url", "": mState.Add "paused", True
        Persist
    End If
    ConfigureSharedProfile cfg
    mHasSnapshot = Records.Count > 0
    If mState.Exists("snapshot_received") Then mHasSnapshot = CBool(mState("snapshot_received"))
    mReady = True
    ' Unshared books (a manual Connect) have no owner password to ask for.
    mActivated = Not SharedProfile(cfg) Or Len(ReadCredential(mWorkbookId)) > 0
    ShowViews mActivated
    RenderGrid
    If Not mActivated Then
        RequireActivation
    ElseIf mState("paused") Or Len(ReadCredential(mWorkbookId)) = 0 Then
        Status Ui("\u0414\u043b\u044f \u044d\u0442\u043e\u0433\u043e \u0448\u0430\u0431\u043b\u043e\u043d\u0430 \u043d\u0443\u0436\u043d\u043e \u043d\u0430\u0441\u0442\u0440\u043e\u0438\u0442\u044c \u043e\u0431\u0449\u0435\u0435 \u043f\u043e\u0434\u043a\u043b\u044e\u0447\u0435\u043d\u0438\u0435")
    Else
        Status Ui("\u041f\u043e\u0434\u043a\u043b\u044e\u0447\u0435\u043d\u0438\u0435 \u043a \u0441\u0435\u0440\u0432\u0435\u0440\u0443\u2026")
        Schedule 1
    End If
    Exit Sub
Failed:
    Dim startupError As String
    startupError = Err.Description
    mReady = False
    On Error Resume Next
    If mLockFile <> 0 Then Close #mLockFile
    mLockFile = 0
    ' Without sync the book still opens for an activated computer (or an
    ' unshared book); a new computer keeps seeing only the lock sheet.
    If Not Gated() Then
        mActivated = True
    ElseIf Len(mWorkbookId) > 0 Then
        mActivated = Len(ReadCredential(mWorkbookId)) > 0
    End If
    If mActivated Then ShowViews True
    UnicodeMsgBox "Synchronization could not start: " & startupError, vbExclamation
End Sub

Private Sub Persist()
    If mState Is Nothing Then Exit Sub
    mSnapshotMerged = False
    On Error GoTo Failed
    AtomicWriteUtf8 mDirectory & "\state.json", JsonStringify(mState)
    Exit Sub
Failed:
    Dim message As String
    message = Err.Description
    CancelTimer
    Err.Raise 5, , "Local changes could not be persisted: " & message
End Sub

' Excel ignores sheet visibility changed inside BeforeSave, so a save that
' would still show receipts is cancelled and repeated here, with events off,
' after they are hidden. A book that already shows only the lock sheet is
' saved by Excel itself.
Public Sub HandleBeforeSave(ByVal saveAsUI As Boolean, ByRef cancel As Boolean)
    Dim target As Variant, hadEvents As Boolean, done As Boolean, message As String
    If IsHidden(ContentSheets()) Then
        cancel = Not PersistBeforeSave()
        Exit Sub
    End If
    cancel = True
    If saveAsUI Then
        target = Application.GetSaveAsFilename(ThisWorkbook.FullName, "Excel Macro-Enabled Workbook (*.xlsm), *.xlsm")
        If VarType(target) = vbBoolean Then Exit Sub
    End If
    If Not PersistBeforeSave() Then Exit Sub
    hadEvents = Application.EnableEvents
    On Error GoTo Failed
    Application.EnableEvents = False
    If saveAsUI Then
        ThisWorkbook.SaveAs Filename:=CStr(target), FileFormat:=52
    Else
        ThisWorkbook.Save
    End If
    done = True
Finished:
    On Error Resume Next
    Application.EnableEvents = hadEvents
    AfterSaved done
    On Error GoTo 0
    If Len(message) > 0 Then UnicodeMsgBox Ui("\u041a\u043d\u0438\u0433\u0430 \u043d\u0435 \u0441\u043e\u0445\u0440\u0430\u043d\u0435\u043d\u0430: ") & message, vbExclamation
    Exit Sub
Failed:
    message = Err.Description
    Resume Finished
End Sub

Public Function PersistBeforeSave() As Boolean
    Dim view As Object, message As String
    ' Even without a running sync the file on disk never shows receipts.
    If Not mReady Then PersistBeforeSave = LockBeforeSave(): Exit Function
    On Error GoTo Failed
    mLastSaveSuccess = False
    CancelTimer
    Persist
    ' Saved files are locked. Without macros they cannot accept untracked edits.
    Set view = BeginViewUpdate()
    Grid.Unprotect LAYOUT_PASSWORD
    Grid.Cells.Locked = True
    Grid.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True
    BankViews.SetEditable False
    mInputsReady = False
    EndViewUpdate view
    Set view = Nothing
    ShowViews False
    PersistBeforeSave = True
    Exit Function
Failed:
    message = Err.Description
    EndViewUpdate view
    CancelTimer
    Status Ui("\u041d\u0435 \u0443\u0434\u0430\u043b\u043e\u0441\u044c \u0441\u043e\u0445\u0440\u0430\u043d\u0438\u0442\u044c \u043e\u0447\u0435\u0440\u0435\u0434\u044c \u043f\u0440\u0430\u0432\u043e\u043a: ") & message
    PersistBeforeSave = False
End Function

Private Function LockBeforeSave() As Boolean
    On Error GoTo Failed
    ShowViews False
    LockBeforeSave = True
    Exit Function
Failed:
    UnicodeMsgBox Ui("\u041d\u0435 \u0443\u0434\u0430\u043b\u043e\u0441\u044c \u0441\u043a\u0440\u044b\u0442\u044c \u0447\u0435\u043a\u0438 \u043f\u0435\u0440\u0435\u0434 \u0441\u043e\u0445\u0440\u0430\u043d\u0435\u043d\u0438\u0435\u043c: ") & Err.Description, vbExclamation
End Function

Public Sub AfterSaved(Optional ByVal success As Boolean = False)
    mLastSaveSuccess = success
    If mActivated Then
        On Error Resume Next
        ShowViews True
        On Error GoTo 0
    End If
    If mReady Then
        ProtectInputs
        BankViews.SetEditable ViewCanEdit()
        If Not mBusy Then
            If Intents.Count > 0 Then Schedule 5 Else Schedule 60
        End If
    End If
    ' Restoring runtime editability changes protection only; the successful save
    ' already persisted user data and a locked on-disk view.
    If success Then ThisWorkbook.Saved = True
End Sub

Public Sub RowAction(ByVal row As Long)
    If Not mReady Or mBusy Or row < FIRST_ROW Then Exit Sub
    mSelectedKey = RowKey(row)
    If Not Records.Exists(mSelectedKey) Then Exit Sub
    Grid.Cells(row, 50).Select
    If Blocked.Exists(mSelectedKey) Then ResolveSelected Else DeleteSelected
End Sub

Public Sub SelectionChanged(ByVal target As Range)
    If Not mReady Or mWriting Then Exit Sub
    mSelectedKey = RowKey(target.row)
End Sub

Private Function SelectedKey() As String
    If ActiveSheet.Name = DataSheetName() Then mSelectedKey = RowKey(ActiveCell.row)
    SelectedKey = mSelectedKey
End Function

Public Sub StopSync()
    On Error Resume Next
    CancelTimer
    CancelPrompt
    ClearSnapshotCache
    mReady = False
    If mLockFile <> 0 Then Close #mLockFile
    mLockFile = 0
    Application.StatusBar = False
End Sub

Public Function PrepareClose() As Boolean
    Dim message As String
    If Not mReady Then PrepareClose = True: Exit Function
    On Error GoTo Failed
    Persist
    SaveView
    StopSync
    ThisWorkbook.Saved = True
    PrepareClose = True
    Exit Function
Failed:
    message = Err.Description
    On Error Resume Next
    Call AfterSaved: Schedule 60
    UnicodeMsgBox Ui("\u041a\u043d\u0438\u0433\u0430 \u043e\u0441\u0442\u0430\u0451\u0442\u0441\u044f \u043e\u0442\u043a\u0440\u044b\u0442\u043e\u0439: ") & message, vbExclamation
End Function

Private Sub SaveView()
    Dim hadEvents As Boolean, message As String, view As Object
    If ThisWorkbook.ReadOnly Then Err.Raise 5, , "The file is read-only; local changes remain queued"
    ' Every caller persists its queue first. A status-only poll leaves Saved
    ' true and must not lock/unlock or rewrite the same file every minute.
    If ThisWorkbook.Saved Then Exit Sub
    hadEvents = Application.EnableEvents
    On Error GoTo Failed
    Set view = BeginViewUpdate()
    ' Excel does not reliably dispatch nested AfterSave from BeforeClose.
    ' Use the same save guards explicitly and verify Excel's actual saved flag.
    If Not PersistBeforeSave() Then Err.Raise 5, , "Local changes could not be persisted before saving"
    Application.EnableEvents = False
    ThisWorkbook.Saved = False
    ThisWorkbook.Save
    If Not ThisWorkbook.Saved Then Err.Raise 5, , "Saving the workbook was cancelled or failed"
    AfterSaved True
    Application.EnableEvents = hadEvents
    EndViewUpdate view
    BankViews.SetEditable ViewCanEdit()
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    AfterSaved False
    Application.EnableEvents = hadEvents
    EndViewUpdate view
    BankViews.SetEditable ViewCanEdit()
    On Error GoTo 0
    Err.Raise 5, , message
End Sub

Private Sub CancelTimer()
    If mDue = 0 Then Exit Sub
    On Error Resume Next
    Application.OnTime EarliestTime:=mDue, Procedure:=mScheduledProcedure, Schedule:=False
    mDue = 0
    mScheduledProcedure = vbNullString
End Sub

Private Sub Schedule(ByVal seconds As Long)
    CancelTimer
    If Not mReady Then Exit Sub
    If mState("paused") Then Exit Sub
    mDue = Now + TimeSerial(0, 0, seconds)
    mScheduledProcedure = "'" & Replace$(ThisWorkbook.Name, "'", "''") & "'!ExcelSync.SyncNow"
    Application.OnTime mDue, mScheduledProcedure
End Sub

Private Function ValidServerUrl(ByVal url As String) As Boolean
    Dim pattern As Object
    Set pattern = CreateObject("VBScript.RegExp")
    pattern.IgnoreCase = True
    pattern.Pattern = "^https://[a-z0-9.-]+(:[0-9]+)?$|^http://(127\.0\.0\.1|localhost)(:[0-9]+)?$"
    ValidServerUrl = pattern.Test(url)
End Function

Public Sub Connect()
    Dim url As String, token As String, answer As Object
    If Not mReady Then StartSync
    If Not mReady Then Exit Sub
    url = Trim$(InputBox("Server address (https://...).", "Connect", TextValue(mState("server_url"))))
    If Len(url) = 0 Then Exit Sub
    Do While Right$(url, 1) = "/": url = Left$(url, Len(url) - 1): Loop
    If Not ValidServerUrl(url) Then UnicodeMsgBox "Use HTTPS, or localhost for a local test.", vbExclamation: Exit Sub
    If Len(TextValue(mState("server_url"))) > 0 Then
        If url <> TextValue(mState("server_url")) And Records.Count > 0 Then
            UnicodeMsgBox "Use a new workbook for another server. Existing records and pending changes are bound to this server.", vbExclamation
            Exit Sub
        End If
    End If
    token = Trim$(InputBox("Paste the grant created for workbook " & mWorkbookId & ". It is stored only in Windows Credential Manager.", "Excel grant"))
    If Len(token) = 0 Then Exit Sub
    On Error GoTo Failed
    Set answer = Request("GET", "/api/excel/v1/snapshot", "", token, url, False)
    CheckIdentity answer
    SaveCredential mWorkbookId, token
    token = vbNullString
    mState("server_url") = url: mState("paused") = False
    mState("owner_id") = TextValue(answer("user_id"))
    mState("device_bound") = False: mBindTried = False
    Persist
    mActivated = True
    ShowViews True
    SyncNow
    Exit Sub
Failed:
    token = vbNullString
    UnicodeMsgBox Err.Description, vbExclamation
End Sub

Public Sub Disconnect()
    If Not mReady Then Exit Sub
    On Error GoTo Failed
    Call CancelTimer: ClearSnapshotCache: DeleteCredential mWorkbookId
    mState("paused") = True: mState("device_bound") = False: Persist
    mBindTried = False
    If Gated() Then mActivated = False: ShowViews False
    Status "Disconnected. Local edits and pending operations are retained."
    Exit Sub
Failed:
    UnicodeMsgBox Err.Description, vbExclamation
End Sub

Private Function Request(ByVal method As String, ByVal path As String, Optional ByVal body As String = "", Optional ByVal candidateToken As String = "", Optional ByVal candidateUrl As String = "", Optional ByVal pauseOnAuthError As Boolean = True) As Object
    Dim http As Object, token As String, url As String, code As Long, device As String, detail As String
    Dim conditional As Boolean, snapshotRequest As Boolean, responseTag As String, parsed As Object
    url = candidateUrl
    If Len(url) = 0 Then url = TextValue(mState("server_url"))
    If Not ValidServerUrl(url) Then Err.Raise 5, , "Connect to a valid server first"
    token = candidateToken
    If Len(token) = 0 Then token = ReadCredential(mWorkbookId)
    If Len(token) = 0 Then Err.Raise 5, , "Reconnect: no Excel grant in Credential Manager"
    Set http = CreateObject("WinHttp.WinHttpRequest.5.1")
    http.SetTimeouts 3000, 3000, 5000, 15000
    http.Option(6) = False ' Never forward credentials through redirects.
    http.Open method, url & path, False
    http.SetRequestHeader "Authorization", "Bearer " & token
    http.SetRequestHeader "X-Excel-Workbook", mWorkbookId
    device = DeviceIdOrBlank()
    If Len(device) > 0 Then http.SetRequestHeader "X-Excel-Device", device
    http.SetRequestHeader "Content-Type", "application/json; charset=utf-8"
    snapshotRequest = (method = "GET" And path = "/api/excel/v1/snapshot")
    mSnapshotUnchanged = False
    If snapshotRequest And mSnapshotUrl = url And Len(mSnapshotTag) > 0 Then
        If Not mSnapshotCache Is Nothing Then
            conditional = True
            http.SetRequestHeader "If-None-Match", mSnapshotTag
        End If
    End If
    If method <> "GET" Then mSnapshotMerged = False
    token = vbNullString
    http.Send body
    code = http.Status
    If code = 401 Or code = 403 Then
        ClearSnapshotCache
        detail = ResponseDetail(http)
        If pauseOnAuthError Then mState("paused") = True: Persist: CancelTimer
        ' This computer's key is gone for good: revoked, expired, or made for
        ' another computer/workbook. A shared book then asks for the password.
        If code = 401 Or InStr(detail, "excel_connection_belongs_to") = 1 Or InStr(detail, "reconnect_excel") > 0 Then
            Err.Raise REVOKED_ERROR, , "Access expired or revoked (HTTP " & code & ")"
        End If
        Err.Raise 5, , "Access expired or revoked (HTTP " & code & "). Use Connect with a new grant."
    End If
    If code = 304 Then
        If Not conditional Then Err.Raise 5, , "Unexpected unchanged snapshot without a local cache"
        responseTag = http.GetResponseHeader("ETag")
        If responseTag <> mSnapshotTag Then Err.Raise 5, , "Unchanged snapshot tag does not match the local cache"
        mSnapshotUnchanged = True
        Set Request = mSnapshotCache
        Exit Function
    End If
    If code < 200 Or code >= 300 Then Err.Raise 5, , "Server returned HTTP " & code
    ' JSON is UTF-8 even when the response Content-Type omits charset.
    ' WinHTTP ResponseText can otherwise decode Cyrillic as Windows-1252.
    Set parsed = JsonParse(DecodeUtf8(http.ResponseBody))
    If snapshotRequest Then
        ClearSnapshotCache
        ' Older servers have no ETag; their full-snapshot behavior remains valid.
        On Error Resume Next
        responseTag = http.GetResponseHeader("ETag")
        On Error GoTo 0
        If Len(responseTag) > 0 Then
            Set mSnapshotCache = parsed: mSnapshotTag = responseTag: mSnapshotUrl = url
        End If
    End If
    Set Request = parsed
End Function

Private Sub ClearSnapshotCache()
    Set mSnapshotCache = Nothing
    mSnapshotTag = vbNullString: mSnapshotUrl = vbNullString
    mSnapshotMerged = False: mSnapshotUnchanged = False
End Sub

Private Sub CheckIdentity(ByVal snapshot As Object)
    Dim owner As String, key As Variant
    If Not snapshot.Exists("user_id") Then Err.Raise IDENTITY_ERROR, , "The server must identify the Excel grant owner before synchronization"
    owner = TextValue(snapshot("user_id"))
    If Len(owner) = 0 Then Err.Raise IDENTITY_ERROR, , "The server returned an empty grant owner"
    If Len(TextValue(ThisWorkbook.Worksheets("_sync").Range("B7").Value2)) > 0 Then
        If owner <> TextValue(ThisWorkbook.Worksheets("_sync").Range("B7").Value2) Then Err.Raise IDENTITY_ERROR, , "The server owner does not match this shared workbook"
    End If
    If mState.Exists("owner_id") Then
        If Len(TextValue(mState("owner_id"))) > 0 Then
            If owner <> TextValue(mState("owner_id")) Then Err.Raise IDENTITY_ERROR, , "This workbook belongs to another account. Use a new workbook; its pending operations cannot be sent under another user."
            Exit Sub
        End If
    End If
    If Outbox.Count > 0 Then Err.Raise IDENTITY_ERROR, , "A legacy pending queue has no account binding. Do not replay it with a new account."
    For Each key In Records.Keys
        If Len(TextValue(Records.Item(key)("id"))) > 0 Then Err.Raise IDENTITY_ERROR, , "Cached server records have no account binding. Use a fresh workbook."
    Next
End Sub

Private Function ColumnField(ByVal column As Long) As String
    Select Case column
        Case 17: ColumnField = "transaction_date"
        Case 20: ColumnField = "transaction_type"
        Case 21: ColumnField = "operator_raw"
        Case 23: ColumnField = "application_mapped"
        Case 26: ColumnField = "amount"
        Case 33: ColumnField = "balance_after"
        Case 34: ColumnField = "card_last_4"
        Case 35: ColumnField = "is_p2p"
        Case 37: ColumnField = "annotation_details"
        Case 38: ColumnField = "annotation_person_code"
        Case 39: ColumnField = "annotation_period"
        Case 40: ColumnField = "annotation_article_code"
        Case 41: ColumnField = "currency"
        Case 42: ColumnField = "receiver_name"
        Case 43: ColumnField = "receiver_card"
        Case 44: ColumnField = "raw_message"
        Case 54: ColumnField = "p2p_confirmed"
    End Select
End Function

Private Function DecimalText(ByVal value As Variant) As String
    Dim text As String
    text = CStr(CDec(value))
    DecimalText = Replace$(text, Mid$(CStr(1.5), 2, 1), ".")
End Function

Private Function DateText(ByVal value As Variant, Optional ByVal monthOnly As Boolean = False) As Variant
    If Len(CStr(value)) = 0 Then DateText = Null: Exit Function
    If Not IsDate(value) And Not IsNumeric(value) Then Err.Raise 5, , "Enter a valid Excel date"
    Dim day As Date
    day = CDate(value)
    If monthOnly Then
        DateText = Format$(DateSerial(Year(day), Month(day), 1), "yyyy-mm-dd")
    Else
        DateText = Format$(day, "yyyy-mm-dd") & "T" & Format$(day, "hh:nn:ss")
    End If
End Function

Private Function OperationEnum(ByVal text As String) As String
    Select Case UCase$(Trim$(text))
        Case "DEBIT", "CREDIT", "CONVERSION", "REVERSAL": OperationEnum = UCase$(Trim$(text))
    End Select
End Function

Private Function ReadOperationKind(ByVal entered As String, ByVal fields As Object, ByVal metadata As Object) As String
    ReadOperationKind = OperationEnum(entered)
    If Len(ReadOperationKind) > 0 Then Exit Function
    ' Copying the displayed bank headline back into T does not edit its type.
    If StrComp(entered, OperationDisplay(fields, metadata), vbBinaryCompare) = 0 Then
        ReadOperationKind = TextValue(fields("transaction_type"))
        Exit Function
    End If
    Err.Raise 5, , "Choose DEBIT, CREDIT, CONVERSION or REVERSAL to change the operation type"
End Function

Private Function AmountForOperation(ByVal value As Variant, ByVal enteredType As String, ByVal effectiveType As String) As String
    Dim kind As String
    If Len(Trim$(TextValue(value))) = 0 Or Not IsNumeric(value) Then Err.Raise 5, , "Amount must be a number"
    kind = OperationEnum(enteredType)
    If Len(kind) = 0 Then kind = effectiveType
    value = Abs(CDec(value))
    If kind = "DEBIT" Then value = -value
    AmountForOperation = DecimalText(value)
End Function

Private Function ReadField(ByVal row As Long, ByVal column As Long, ByVal fields As Object, ByVal metadata As Object) As Variant
    Dim value As Variant, text As String
    If Grid.Cells(row, column).HasFormula Then Err.Raise 5, , "Enter a value, not a formula, in editable cells"
    value = Grid.Cells(row, column).Value2
    If IsError(value) Then Err.Raise 5, , "Cell contains an Excel error"
    text = Trim$(CStr(value))
    Select Case ColumnField(column)
        Case "transaction_date": ReadField = DateText(value)
        Case "annotation_period": ReadField = DateText(value, True)
        Case "amount"
            ' T normally contains a bank headline, not a canonical enum. Only
            ' an explicit enum in a simultaneous paste overrides effective type.
            ReadField = AmountForOperation(value, TextValue(Grid.Cells(row, 20).Value2), TextValue(fields("transaction_type")))
        Case "balance_after"
            If Len(text) = 0 Then ReadField = Null Else ReadField = DecimalText(value)
        Case "is_p2p"
            Select Case LCase$(text)
                Case "1", "true", "yes": ReadField = True
                Case "", "0", "false", "no": ReadField = False
                Case Else: Err.Raise 5, , "P2P must be 0 or 1"
            End Select
        Case "p2p_confirmed"
            Select Case LCase$(text)
                Case "": ReadField = Null
                Case "1", "true", "yes": ReadField = True
                Case "0", "false", "no": ReadField = False
                Case Else: Err.Raise 5, , "P2P confirmation must be blank, 0 or 1"
            End Select
        Case "transaction_type": ReadField = ReadOperationKind(CStr(value), fields, metadata)
        Case "currency": ReadField = UCase$(text)
        Case "raw_message": ReadField = CStr(value)
        Case Else
            If Len(text) = 0 Then ReadField = Null Else ReadField = CStr(value)
    End Select
End Function

Private Function StateFieldValue(ByVal fields As Object, ByVal field As String) As Variant
    If fields.Exists(field) Then StateFieldValue = fields(field) Else StateFieldValue = Null
End Function

Private Function CompleteFields(ByVal fields As Object) As Object
    Dim result As Object
    Set result = JsonClone(fields)
    ' Old snapshots/ACKs predate these nullable fields. Never modify immutable
    ' outbox payloads when completing a read-only view or constructing a NEW op.
    If Not result.Exists("excel_bank") Then result.Add "excel_bank", Null
    If Not result.Exists("p2p_confirmed") Then result.Add "p2p_confirmed", Null
    Set CompleteFields = result
End Function

Private Function EffectiveFields(ByVal key As String) As Object
    Dim field As Variant, result As Object
    Set result = CompleteFields(Records.Item(key)("fields"))
    If Intents.Exists(key) Then
        For Each field In Intents.Item(key)("fields").Keys
            result(field) = Intents.Item(key)("fields")(field)
        Next
    End If
    Set EffectiveFields = result
End Function

Public Sub CellsEdited(ByVal changed As Range, Optional ByVal bankName As String = "")
    Dim area As Range, cell As Range, field As String, key As String
    Dim touched As Object, rowKeyValue As Variant, row As Long, intent As Object, col As Long, columns As Object, columnKey As Variant, staged As Object
    Dim additions As Object, newRows As Object, record As Object, inputRow As Long, currentFields As Object
    If Not mReady Or mWriting Then Exit Sub
    On Error GoTo Failed
    Set area = Intersect(changed, Grid.Range("O5:BB" & Grid.Rows.Count))
    If area Is Nothing Then Exit Sub
    Set staged = JsonClone(Intents)
    Set touched = Dict(): Set additions = Dict(): Set newRows = Dict()
    inputRow = Grid.Cells(Grid.Rows.Count, UUID_COL).End(xlUp).row + 1
    If inputRow < FIRST_ROW Then inputRow = FIRST_ROW
    For Each cell In area.Cells
        field = ColumnField(cell.Column)
        If Len(field) > 0 Then
            key = RowKey(cell.row)
            If Not Records.Exists(key) Then
                If cell.row <> inputRow Or Not mHasSnapshot Then Err.Raise 5, , "Wait for the first connection, then use the empty row below the receipts"
                If Not newRows.Exists(CStr(cell.row)) Then
                    Set record = Dict(): record.Add "id", "": record.Add "uuid", NewUuid(): record.Add "version", ""
                    record.Add "fields", EmptyFields(): record.Add "metadata", Dict()
                    key = KeyOf(record): additions.Add key, record: newRows.Add CStr(cell.row), key
                    Set intent = Dict(): intent.Add "kind", "draft": intent.Add "fields", Dict(): staged.Add key, intent
                    If Len(bankName) > 0 Then intent("fields").Add "excel_bank", bankName
                Else
                    key = CStr(newRows(CStr(cell.row)))
                End If
            End If
            If Not touched.Exists(key) Then touched.Add key, Dict()
            Set columns = touched(key)
            columns(CStr(cell.Column)) = cell.row
        End If
    Next
    ' Capture only edited columns. The server changes amount sign for a type edit.
    ' Untouched values may have more precision than their Excel display.
    For Each rowKeyValue In touched.Keys
        key = CStr(rowKeyValue): Set columns = touched(key)
        If Not staged.Exists(key) Then
            Set intent = Dict(): intent.Add "kind", "update": intent.Add "fields", Dict()
            staged.Add key, intent
        End If
        Set intent = staged(key)
        If intent("kind") = "delete" Then Err.Raise 5, , "This row is pending deletion"
        If intent.Exists("awaiting_correction") Then intent.Remove "awaiting_correction"
        If additions.Exists(key) Then Set record = additions(key) Else Set record = Records.Item(key)
        If Intents.Exists(key) Then Set currentFields = EffectiveFields(key) Else Set currentFields = record("fields")
        For Each columnKey In columns.Keys
            col = CLng(columnKey): row = CLng(columns(columnKey))
            field = ColumnField(col)
            If Len(field) > 0 Then
                Dim value As Variant, old As Variant
                If intent("kind") = "draft" And field = "amount" And Len(TextValue(Grid.Cells(row, col).Value2)) = 0 Then
                    value = Null
                Else
                    value = ReadField(row, col, currentFields, record("metadata"))
                End If
                old = StateFieldValue(record("fields"), field)
                If field = "raw_message" And Len(TextValue(old)) > 32767 Then
                    If UnicodeMsgBox("The original text is longer than an Excel cell. Replace the entire server text with this edited cell?", vbYesNo + vbExclamation + vbDefaultButton2) <> vbYes Then Err.Raise 5, , "Original long text left unchanged"
                End If
                If SameValue(value, old) And Not HasOperation(key) Then
                    If intent("fields").Exists(field) Then intent("fields").Remove field
                Else
                    intent("fields")(field) = value
                End If
            End If
        Next
        If intent("fields").Count = 0 And intent("kind") = "update" Then staged.Remove key
        If Blocked.Exists(key) Then Status Ui("\u0414\u0432\u0430\u0436\u0434\u044b \u043d\u0430\u0436\u043c\u0438\u0442\u0435 \u0441\u043e\u0441\u0442\u043e\u044f\u043d\u0438\u0435 \u0441\u0442\u0440\u043e\u043a\u0438, \u0447\u0442\u043e\u0431\u044b \u0440\u0430\u0437\u0440\u0435\u0448\u0438\u0442\u044c \u043a\u043e\u043d\u0444\u043b\u0438\u043a\u0442")
    Next
    For Each rowKeyValue In additions.Keys
        Records.Add CStr(rowKeyValue), additions(rowKeyValue)
    Next
    Set mState("intents") = staged
    Persist
    RenderGrid
    Status Ui("\u041f\u0440\u0430\u0432\u043a\u0438 \u0441\u043e\u0445\u0440\u0430\u043d\u0435\u043d\u044b \u043d\u0430 \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440\u0435. \u041e\u0442\u043f\u0440\u0430\u0432\u043a\u0430 \u0430\u0432\u0442\u043e\u043c\u0430\u0442\u0438\u0447\u0435\u0441\u043a\u0438\u2026")
    Schedule 5
    Exit Sub
Failed:
    Dim message As String
    message = Err.Description
    On Error Resume Next
    Persist
    RenderGrid
    UnicodeMsgBox message, vbExclamation
End Sub

Private Function DraftReady(ByVal fields As Object) As Boolean
    Dim required As Variant, field As Variant
    required = Array("transaction_date", "transaction_type", "operator_raw", "amount", "currency", "card_last_4")
    For Each field In required
        If Len(Trim$(TextValue(fields(field)))) = 0 Then Exit Function
    Next
    If Val(TextValue(fields("amount"))) = 0 Then Exit Function
    Select Case TextValue(fields("transaction_type"))
        Case "DEBIT", "CREDIT", "CONVERSION", "REVERSAL"
        Case Else: Exit Function
    End Select
    Select Case TextValue(fields("currency"))
        Case "UZS", "USD", "EUR", "RUB", "KZT", "GBP", "CNY"
        Case Else: Exit Function
    End Select
    DraftReady = True
End Function

Private Sub PromoteDrafts()
    Dim key As Variant, fields As Object
    For Each key In Intents.Keys
        If Intents.Item(key)("kind") = "draft" And Not Blocked.Exists(key) And Not Intents.Item(key).Exists("awaiting_correction") Then
            Set fields = EffectiveFields(CStr(key))
            If DraftReady(fields) Then
                Intents.Item(key)("kind") = "create"
                Set Intents.Item(key)("fields") = fields
            End If
        End If
    Next
End Sub

Private Sub BuildOperations()
    Dim key As Variant, intent As Object, record As Object, op As Object, base As Object, field As Variant
    Dim added As Boolean
    For Each key In Intents.Keys
        Set intent = Intents.Item(key)
        If intent("kind") <> "draft" And Not Blocked.Exists(key) And Not HasOperation(CStr(key)) Then
            Set record = Records.Item(key): Set op = Dict(): Set base = Dict()
            op.Add "operation_id", NewUuid(): op.Add "kind", intent("kind")
            If intent("kind") = "create" Then
                op.Add "_local_key", CStr(key)
            Else
                op.Add "id", record("id"): op.Add "uuid", record("uuid")
            End If
            If intent("kind") = "delete" Then
                Set base = CompleteFields(record("fields"))
            ElseIf intent("kind") = "update" Then
                For Each field In intent("fields").Keys: base.Add field, StateFieldValue(record("fields"), CStr(field)): Next
            End If
            op.Add "base", base: op.Add "fields", JsonClone(intent("fields"))
            Outbox.Add op("operation_id"), op
            added = True
        End If
    Next
    ' IDs and payloads are immutable before the first network attempt.
    ' Persist every new immutable payload before any network attempt. Existing
    ' intentions can also have been promoted from draft during this cycle.
    ' An empty queue has nothing to rebuild or persist on an ordinary poll.
    If added Or Intents.Count > 0 Then Persist
End Sub

Public Sub SyncNow()
    Dim body As Object, operations As New Collection, id As Variant, response As Object, result As Variant, snapshot As Object, wire As Object
    Dim cycleToken As String, cycleUrl As String, snapshotChanged As Boolean, failure As Long
    If Not mReady Then StartSync
    If Not mReady Or mBusy Then Exit Sub
    If Not mActivated Then RequireActivation False: Exit Sub
    ' Keep the user's Excel copy/cut operation intact; the durable queue stays
    ' in place and the next timer retries after copy mode has ended.
    If Application.CutCopyMode <> False Then Schedule 5: Exit Sub
    If mState("paused") Then Status Ui("\u0414\u043e\u0441\u0442\u0443\u043f \u043f\u0440\u0438\u043e\u0441\u0442\u0430\u043d\u043e\u0432\u043b\u0435\u043d. \u041d\u0443\u0436\u043d\u0430 \u043a\u043d\u0438\u0433\u0430 \u0441 \u0434\u0435\u0439\u0441\u0442\u0432\u0443\u044e\u0449\u0438\u043c \u043f\u043e\u0434\u043a\u043b\u044e\u0447\u0435\u043d\u0438\u0435\u043c."): Exit Sub
    mBusy = True: CancelTimer
    On Error GoTo Failed
    Status Ui("\u0421\u0438\u043d\u0445\u0440\u043e\u043d\u0438\u0437\u0430\u0446\u0438\u044f\u2026")
    ' Authenticate the principal before replaying even one mutation. This also
    ' protects a grant replaced outside this workbook in Credential Manager.
    cycleToken = ReadCredential(mWorkbookId): cycleUrl = TextValue(mState("server_url"))
    If Len(cycleToken) = 0 Then Err.Raise REVOKED_ERROR, , "The shared connection key is unavailable"
    ' A computer that already had the old shared key keeps working: trade it
    ' once for a key that works only on this computer.
    If Not mBindTried And Not DeviceBound() Then
        mBindTried = True
        cycleToken = BindDevice(cycleToken)
    End If
    ' Use this exact credential for the whole cycle: another process may update
    ' WinCred between GET and POST, but an unverified owner must never be used.
    Set snapshot = Request("GET", "/api/excel/v1/snapshot", "", cycleToken, cycleUrl)
    CheckIdentity snapshot
    If Not mState.Exists("owner_id") Then
        mState("owner_id") = TextValue(snapshot("user_id"))
        Persist
    ElseIf TextValue(mState("owner_id")) <> TextValue(snapshot("user_id")) Then
        mState("owner_id") = TextValue(snapshot("user_id"))
        Persist
    End If
    PromoteDrafts
    BuildOperations
    For Each id In Outbox.Keys
        If Not Blocked.Exists(KeyOf(Outbox.Item(id))) Then
            Set wire = JsonClone(Outbox.Item(id))
            If wire.Exists("_local_key") Then wire.Remove "_local_key"
            operations.Add wire
            If operations.Count >= 100 Then Exit For
        End If
    Next
    If operations.Count > 0 Then
        Set body = Dict(): body.Add "workbook_id", mWorkbookId: body.Add "operations", operations
        Set response = Request("POST", "/api/excel/v1/sync", JsonStringify(body), cycleToken, cycleUrl)
        For Each result In response("results"): AcceptResult result: Next
        Persist
        Set snapshot = Request("GET", "/api/excel/v1/snapshot", "", cycleToken, cycleUrl)
        CheckIdentity snapshot
    End If
    ' Only reveal new server UUIDs after acknowledgements have remapped drafts.
    ' Otherwise a committed create with a lost reply could appear twice.
    ' A conditional response is still authenticated and scoped by the server.
    ' Reuse it only after a successful merge with no local work to reconcile.
    If Not mSnapshotUnchanged Or Not mSnapshotMerged Or Intents.Count > 0 Or Outbox.Count > 0 Or Blocked.Count > 0 Then
        snapshotChanged = MergeSnapshot(snapshot)
    End If
    mHasSnapshot = True
    If Not mState.Exists("snapshot_received") Then
        mState("snapshot_received") = True: snapshotChanged = True
    ElseIf Not CBool(mState("snapshot_received")) Then
        mState("snapshot_received") = True: snapshotChanged = True
    End If
    If snapshotChanged Then Persist
    RenderGrid
    Status Ui("\u041e\u0431\u043d\u043e\u0432\u043b\u0435\u043d\u043e ") & Format$(Now, "hh:nn:ss") & Ui(". \u041e\u0436\u0438\u0434\u0430\u044e\u0442 \u043e\u0442\u043f\u0440\u0430\u0432\u043a\u0438: ") & Intents.Count & Ui("; \u0442\u0440\u0435\u0431\u0443\u044e\u0442 \u0432\u043d\u0438\u043c\u0430\u043d\u0438\u044f: ") & Blocked.Count
    If HasUnreconciledCreate() Then Status Ui("\u041e\u0436\u0438\u0434\u0430\u0435\u0442\u0441\u044f \u043f\u043e\u0434\u0442\u0432\u0435\u0440\u0436\u0434\u0435\u043d\u0438\u0435 \u043d\u043e\u0432\u043e\u0433\u043e \u0447\u0435\u043a\u0430. \u041f\u0440\u0438 \u043a\u043e\u043d\u0444\u043b\u0438\u043a\u0442\u0435 \u0434\u0432\u0430\u0436\u0434\u044b \u043d\u0430\u0436\u043c\u0438\u0442\u0435 \u0441\u043e\u0441\u0442\u043e\u044f\u043d\u0438\u0435 \u0441\u0442\u0440\u043e\u043a\u0438.")
    SaveView
    mSnapshotMerged = (Intents.Count = 0 And Outbox.Count = 0 And Blocked.Count = 0)
    cycleToken = vbNullString
    mBusy = False
    BankViews.SetEditable ViewCanEdit()
    Schedule 60
    Exit Sub
Failed:
    cycleToken = vbNullString
    failure = Err.Number
    If failure = IDENTITY_ERROR Then mState("paused") = True
    Status Err.Description & Ui(". \u041f\u0440\u0430\u0432\u043a\u0438 \u0441\u043e\u0445\u0440\u0430\u043d\u0435\u043d\u044b \u043d\u0430 \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440\u0435.")
    mBusy = False
    On Error Resume Next
    Persist
    If failure = REVOKED_ERROR And Gated() Then
        ' Pending edits stay in the local queue and are sent after activation.
        DeleteCredential mWorkbookId
        mActivated = False: mBindTried = False
        ShowViews False
        BankViews.SetEditable False
        RequireActivation True, Ui("\u0414\u043e\u0441\u0442\u0443\u043f \u044d\u0442\u043e\u0433\u043e \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440\u0430 \u043e\u0442\u043e\u0437\u0432\u0430\u043d. ")
        Exit Sub
    End If
    RenderGrid
    SaveView
    BankViews.SetEditable ViewCanEdit()
    Schedule 60
End Sub

Private Sub AcceptResult(ByVal result As Object)
    Dim op As Object, key As String, id As String, state As String, field As Variant, intent As Object, newKey As String
    id = TextValue(result("operation_id"))
    If Not Outbox.Exists(id) Then Exit Sub
    Set op = Outbox.Item(id): key = KeyOf(op): state = TextValue(result("status"))
    If state = "applied" Or state = "noop" Or state = "deleted" Or (state = "not_found" And op("kind") = "delete") Then
        If op("kind") = "delete" Then
            If Records.Exists(key) Then Records.Remove key
            If Intents.Exists(key) Then Intents.Remove key
        Else
            If Not result.Exists("record") Then Err.Raise 5, , "Successful result omitted its record; queue retained"
            newKey = KeyOf(result("record"))
            If newKey <> key Then
                ' A create acknowledgement replaces a draft UUID. Keep that
                ' receipt at its current screen position until the next render.
                If mRowAliases Is Nothing Then Set mRowAliases = Dict()
                mRowAliases(key) = newKey
                If Not mState.Exists("view_aliases") Then mState.Add "view_aliases", Dict()
                mState("view_aliases")(key) = newKey
                If Records.Exists(key) Then Records.Remove key
                If Intents.Exists(key) Then
                    Set intent = Intents.Item(key)
                    Intents.Remove key: Intents.Add newKey, intent
                End If
                If Blocked.Exists(key) Then Blocked.Remove key
                If mSelectedKey = key Then mSelectedKey = newKey
                key = newKey
            End If
            Set Records.Item(key) = JsonClone(result("record"))
            If Intents.Exists(key) Then
                Set intent = Intents.Item(key)
                For Each field In op("fields").Keys
                    If intent("fields").Exists(field) Then
                        If SameValue(intent("fields")(field), op("fields")(field)) Then intent("fields").Remove field
                    End If
                Next
                intent("kind") = "update"
                If intent("fields").Count = 0 Then Intents.Remove key
            End If
        End If
        If Blocked.Exists(key) Then Blocked.Remove key
        Outbox.Remove id
    Else
        If Blocked.Exists(key) Then Blocked.Remove key
        Blocked.Add key, JsonClone(result)
    End If
End Sub

Private Function SameSnapshotValue(ByVal leftValue As Variant, ByVal rightValue As Variant) As Boolean
    Dim key As Variant, index As Long
    ' Compare the complete JSON value, including read-only metadata. The
    ' server's version hashes editable fields only, so it is not sufficient.
    ' No display tolerance or string coercion is safe for persisted state.
    If IsObject(leftValue) Or IsObject(rightValue) Then
        If Not IsObject(leftValue) Or Not IsObject(rightValue) Then Exit Function
        If leftValue Is Nothing Then
            SameSnapshotValue = (rightValue Is Nothing)
            Exit Function
        End If
        If rightValue Is Nothing Then Exit Function
        If TypeName(leftValue) <> TypeName(rightValue) Then Exit Function
        Select Case TypeName(leftValue)
            Case "Collection"
                If leftValue.Count <> rightValue.Count Then Exit Function
                For index = 1 To leftValue.Count
                    If Not SameSnapshotValue(leftValue(index), rightValue(index)) Then Exit Function
                Next
            Case "Dictionary"
                If leftValue.Count <> rightValue.Count Then Exit Function
                For Each key In leftValue.Keys
                    If Not rightValue.Exists(key) Then Exit Function
                    If Not SameSnapshotValue(leftValue(key), rightValue(key)) Then Exit Function
                Next
            Case Else
                Exit Function
        End Select
        SameSnapshotValue = True
    ElseIf IsNull(leftValue) Or IsNull(rightValue) Then
        SameSnapshotValue = IsNull(leftValue) And IsNull(rightValue)
    ElseIf IsEmpty(leftValue) Or IsEmpty(rightValue) Then
        SameSnapshotValue = IsEmpty(leftValue) And IsEmpty(rightValue)
    ElseIf VarType(leftValue) = vbString Or VarType(rightValue) = vbString Then
        If VarType(leftValue) = vbString And VarType(rightValue) = vbString Then
            SameSnapshotValue = (StrComp(leftValue, rightValue, vbBinaryCompare) = 0)
        End If
    ElseIf VarType(leftValue) = vbBoolean Or VarType(rightValue) = vbBoolean Then
        If VarType(leftValue) = vbBoolean And VarType(rightValue) = vbBoolean Then SameSnapshotValue = (leftValue = rightValue)
    ElseIf IsNumeric(leftValue) And IsNumeric(rightValue) Then
        SameSnapshotValue = (leftValue = rightValue)
    End If
End Function

Private Function MergeSnapshot(ByVal snapshot As Object) As Boolean
    Dim seen As Object, record As Variant, key As String, existing As Variant, mayAdd As Boolean
    Set seen = Dict(): mayAdd = Not HasUnreconciledCreate()
    For Each record In snapshot("records")
        key = KeyOf(record): seen(key) = True
        ' A snapshot must not replace the original base of an unsent or retried edit.
        If Not Pending(key) Then
            If Records.Exists(key) Then
                If Not SameSnapshotValue(Records.Item(key), record) Then
                    Set Records.Item(key) = JsonClone(record)
                    MergeSnapshot = True
                End If
            ElseIf mayAdd Then
                Records.Add key, JsonClone(record)
                MergeSnapshot = True
            End If
        End If
    Next
    If snapshot.Exists("complete") Then
        If snapshot("complete") Then
            For Each existing In Records.Keys
                If Not seen.Exists(existing) And Not Pending(CStr(existing)) Then
                    Records.Remove existing
                    MergeSnapshot = True
                End If
            Next
        End If
    End If
    If snapshot.Exists("references") Then UpdateReferences snapshot("references")
End Function

Private Function ProblemText(ByVal problem As Object) As String
    Dim field As Variant, col As Long, caption As String, detail As Object, result As String
    If problem.Exists("conflicts") Then
        For Each field In problem("conflicts").Keys
            caption = CStr(field)
            For col = 17 To 44
                If ColumnField(col) = CStr(field) Then caption = TextValue(Grid.Cells(2, col).Value2): Exit For
            Next
            Set detail = problem("conflicts")(field)
            result = result & caption & vbCrLf & Ui("  \u041d\u0430 \u0441\u0435\u0440\u0432\u0435\u0440\u0435: ") & Left$(TextValue(detail("current")), 100) & vbCrLf & Ui("  \u041c\u043e\u0438 \u043f\u0440\u0430\u0432\u043a\u0438: ") & Left$(TextValue(detail("proposed")), 100) & vbCrLf
        Next
    ElseIf problem.Exists("error") Then
        result = Ui("\u041f\u0440\u0438\u0447\u0438\u043d\u0430: ") & Replace$(TextValue(problem("error")), "_", " ")
    Else
        result = "The server could not accept this change. Check access and the entered values."
    End If
    ProblemText = Left$(result, 650)
End Function

Public Sub ResolveSelected()
    Dim key As String, snapshot As Object, record As Variant, fresh As Object, answer As VbMsgBoxResult, opId As Variant
    If Not mReady Or mBusy Then Exit Sub
    key = SelectedKey()
    If Not Blocked.Exists(key) Then UnicodeMsgBox Ui("\u0412 \u044d\u0442\u043e\u0439 \u0441\u0442\u0440\u043e\u043a\u0435 \u043d\u0435\u0442 \u043a\u043e\u043d\u0444\u043b\u0438\u043a\u0442\u0430."): Exit Sub
    On Error GoTo Failed
    If Records.Exists(key) Then
        If Len(TextValue(Records.Item(key)("id"))) = 0 Then
            If Blocked.Item(key).Exists("applied") Then
                If Blocked.Item(key)("applied") Then
                    If UnicodeMsgBox("This creation was already applied, but the server cannot disclose the receipt now. Restore access and retry the SAME operation ID? A new receipt will not be created.", vbYesNo + vbExclamation) = vbYes Then
                        Blocked.Remove key
                        Call Persist: Schedule 1
                    End If
                    Exit Sub
                End If
            End If
            If Blocked.Item(key)("status") <> "invalid" And Blocked.Item(key)("status") <> "forbidden" Then
                UnicodeMsgBox "This operation has an uncertain result. Keep its ID and reconnect; do not recreate the receipt.", vbExclamation
                Exit Sub
            End If
            answer = UnicodeMsgBox(Ui("\u0421\u0435\u0440\u0432\u0435\u0440 \u043e\u0442\u043a\u043b\u043e\u043d\u0438\u043b \u043d\u043e\u0432\u044b\u0439 \u0447\u0435\u043a. ") & ProblemText(Blocked.Item(key)) & vbCrLf & Ui("\u0412\u0435\u0440\u043d\u0443\u0442\u044c \u0432 \u0447\u0435\u0440\u043d\u043e\u0432\u0438\u043a? \u0418\u0441\u043f\u0440\u0430\u0432\u044c\u0442\u0435 \u043f\u043e\u043b\u044f, \u043f\u043e\u0441\u043b\u0435 \u044d\u0442\u043e\u0433\u043e \u0447\u0435\u043a \u043e\u0442\u043f\u0440\u0430\u0432\u0438\u0442\u0441\u044f \u0430\u0432\u0442\u043e\u043c\u0430\u0442\u0438\u0447\u0435\u0441\u043a\u0438."), vbYesNo + vbQuestion)
            If answer <> vbYes Then Exit Sub
            Set Intents.Item(key)("fields") = EffectiveFields(key)
            Intents.Item(key)("kind") = "draft"
            ' A correction is required before automatic retry of invalid input.
            Intents.Item(key)("awaiting_correction") = True
            GoTo Rebased
        End If
    End If
    Set snapshot = Request("GET", "/api/excel/v1/snapshot")
    CheckIdentity snapshot
    For Each record In snapshot("records")
        If KeyOf(record) = key Then Set fresh = record: Exit For
    Next
    If fresh Is Nothing Then
        If UnicodeMsgBox(Ui("\u0427\u0435\u043a \u0443\u0434\u0430\u043b\u0451\u043d \u0438\u043b\u0438 \u043d\u0435\u0434\u043e\u0441\u0442\u0443\u043f\u0435\u043d \u043d\u0430 \u0441\u0435\u0440\u0432\u0435\u0440\u0435. \u041e\u0442\u0431\u0440\u043e\u0441\u0438\u0442\u044c \u0435\u0433\u043e \u043d\u0435\u043e\u0442\u043f\u0440\u0430\u0432\u043b\u0435\u043d\u043d\u044b\u0435 \u043f\u0440\u0430\u0432\u043a\u0438 \u043d\u0430 \u044d\u0442\u043e\u043c \u043a\u043e\u043c\u043f\u044c\u044e\u0442\u0435\u0440\u0435?"), vbYesNo + vbExclamation) <> vbYes Then Exit Sub
        If Intents.Exists(key) Then Intents.Remove key
        If Records.Exists(key) Then Records.Remove key
    Else
        answer = UnicodeMsgBox(Ui("\u0414\u0410: \u0441\u043e\u0445\u0440\u0430\u043d\u0438\u0442\u044c \u043c\u043e\u0438 \u043f\u0440\u0430\u0432\u043a\u0438 \u043f\u043e\u0432\u0435\u0440\u0445 \u043e\u0431\u043d\u043e\u0432\u043b\u0451\u043d\u043d\u044b\u0445 \u0434\u0430\u043d\u043d\u044b\u0445 \u0441\u0435\u0440\u0432\u0435\u0440\u0430.") & vbCrLf & Ui("\u041d\u0415\u0422: \u043f\u0440\u0438\u043d\u044f\u0442\u044c \u0432\u0441\u0435 \u0437\u043d\u0430\u0447\u0435\u043d\u0438\u044f \u0441 \u0441\u0435\u0440\u0432\u0435\u0440\u0430.") & vbCrLf & Ui("\u041e\u0422\u041c\u0415\u041d\u0410: \u043e\u0441\u0442\u0430\u0432\u0438\u0442\u044c \u043a\u043e\u043d\u0444\u043b\u0438\u043a\u0442 \u0434\u043b\u044f \u0440\u0435\u0448\u0435\u043d\u0438\u044f \u043f\u043e\u0437\u0436\u0435.") & vbCrLf & ProblemText(Blocked.Item(key)), vbYesNoCancel + vbQuestion, Ui("\u041a\u043e\u043d\u0444\u043b\u0438\u043a\u0442 \u0438\u0437\u043c\u0435\u043d\u0435\u043d\u0438\u0439"))
        If answer = vbCancel Then Exit Sub
        Set Records.Item(key) = JsonClone(fresh)
        If answer = vbNo Then
            If Intents.Exists(key) Then Intents.Remove key
        End If
    End If
Rebased:
    ' The rejected operation is never mutated/reused. Explicit rebase gets a new ID.
    For Each opId In Outbox.Keys
        If KeyOf(Outbox.Item(opId)) = key Then Outbox.Remove opId
    Next
    Blocked.Remove key
    Call Persist: RenderGrid: Schedule 1
    Exit Sub
Failed:
    UnicodeMsgBox Err.Description, vbExclamation
End Sub

Private Function EmptyFields() As Object
    Dim fields As Object, col As Long, field As String
    Set fields = Dict()
    For col = 17 To 44
        field = ColumnField(col)
        If Len(field) > 0 Then fields.Add field, Null
    Next
    fields("transaction_date") = Format$(Now, "yyyy-mm-dd") & "T" & Format$(Now, "hh:nn:ss")
    fields("transaction_type") = "DEBIT": fields("currency") = "UZS"
    fields("amount") = Null: fields("is_p2p") = False
    fields("raw_message") = ""
    fields.Add "excel_bank", Null: fields.Add "p2p_confirmed", Null
    Set EmptyFields = fields
End Function

Public Sub AddReceipt()
    Dim record As Object, intent As Object, key As String
    If Not mReady Then StartSync
    If Not mReady Then Exit Sub
    If BankViews.Enabled() Then BankViews.SelectInputRow: Exit Sub
    On Error GoTo Failed
    Set record = Dict(): record.Add "id", "": record.Add "uuid", NewUuid(): record.Add "version", ""
    record.Add "fields", EmptyFields(): record.Add "metadata", Dict()
    key = KeyOf(record): Records.Add key, record
    Set intent = Dict(): intent.Add "kind", "draft": intent.Add "fields", Dict()
    Intents.Add key, intent
    Call Persist: RenderGrid
    Grid.Activate: Grid.Cells(FindRow(key), 21).Select
    Status Ui("\u0427\u0435\u0440\u043d\u043e\u0432\u0438\u043a: \u0437\u0430\u043f\u043e\u043b\u043d\u0438\u0442\u0435 \u0434\u0430\u0442\u0443, \u0442\u0438\u043f, \u043e\u043f\u0435\u0440\u0430\u0442\u043e\u0440\u0430, \u043a\u0430\u0440\u0442\u0443, \u0441\u0443\u043c\u043c\u0443 \u0438 \u0432\u0430\u043b\u044e\u0442\u0443. \u041e\u0442\u043f\u0440\u0430\u0432\u043a\u0430 \u0430\u0432\u0442\u043e\u043c\u0430\u0442\u0438\u0447\u0435\u0441\u043a\u0430\u044f.")
    Exit Sub
Failed:
    UnicodeMsgBox Err.Description, vbExclamation
End Sub

Public Sub CreateReceipt()
    Dim key As String, fields As Object
    If Not mReady Then Exit Sub
    key = SelectedKey()
    If Not Intents.Exists(key) Then Exit Sub
    If Intents.Item(key)("kind") <> "draft" Then Exit Sub
    Set fields = EffectiveFields(key)
    If Not DraftReady(fields) Then
        UnicodeMsgBox Ui("\u0417\u0430\u043f\u043e\u043b\u043d\u0438\u0442\u0435 \u0434\u0430\u0442\u0443, \u0442\u0438\u043f, \u043e\u043f\u0435\u0440\u0430\u0442\u043e\u0440\u0430, \u043a\u0430\u0440\u0442\u0443, \u043d\u0435\u043d\u0443\u043b\u0435\u0432\u0443\u044e \u0441\u0443\u043c\u043c\u0443 \u0438 \u0432\u0430\u043b\u044e\u0442\u0443."), vbExclamation: Exit Sub
    End If
    If UnicodeMsgBox(Ui("\u0421\u043e\u0437\u0434\u0430\u0442\u044c \u044d\u0442\u043e\u0442 \u0447\u0435\u043a \u043d\u0430 \u0441\u0435\u0440\u0432\u0435\u0440\u0435?"), vbYesNo + vbQuestion) <> vbYes Then Exit Sub
    On Error GoTo Failed
    Intents.Item(key)("kind") = "create"
    Set Intents.Item(key)("fields") = fields
    Call Persist: RenderGrid: Schedule 1
    Exit Sub
Failed:
    UnicodeMsgBox Err.Description, vbExclamation
End Sub

Public Sub DeleteSelected()
    Dim key As String, intent As Object
    If Not mReady Then Exit Sub
    key = SelectedKey()
    If Not Records.Exists(key) Then Exit Sub
    If HasOperation(key) Then UnicodeMsgBox Ui("\u0421\u043d\u0430\u0447\u0430\u043b\u0430 \u0434\u043e\u0436\u0434\u0438\u0442\u0435\u0441\u044c \u043e\u0442\u043f\u0440\u0430\u0432\u043a\u0438 \u0438\u043b\u0438 \u0440\u0430\u0437\u0440\u0435\u0448\u0438\u0442\u0435 \u043a\u043e\u043d\u0444\u043b\u0438\u043a\u0442 \u0432 \u044d\u0442\u043e\u0439 \u0441\u0442\u0440\u043e\u043a\u0435."), vbExclamation: Exit Sub
    If UnicodeMsgBox(Ui("\u0423\u0434\u0430\u043b\u0438\u0442\u044c \u044d\u0442\u043e\u0442 \u0447\u0435\u043a \u0441 \u0441\u0435\u0440\u0432\u0435\u0440\u0430? \u041e\u0442\u043c\u0435\u043d\u0438\u0442\u044c \u0443\u0434\u0430\u043b\u0435\u043d\u0438\u0435 \u0432 \u044d\u0442\u043e\u0439 \u043a\u043d\u0438\u0433\u0435 \u043d\u0435\u043b\u044c\u0437\u044f."), vbYesNo + vbExclamation, Ui("\u0423\u0434\u0430\u043b\u0435\u043d\u0438\u0435 \u0447\u0435\u043a\u0430")) <> vbYes Then Exit Sub
    On Error GoTo Failed
    If Len(TextValue(Records.Item(key)("id"))) = 0 Then
        Records.Remove key
        If Intents.Exists(key) Then Intents.Remove key
    Else
        If Intents.Exists(key) Then Intents.Remove key
        Set intent = Dict(): intent.Add "kind", "delete": intent.Add "fields", Dict()
        Intents.Add key, intent
    End If
    If Blocked.Exists(key) Then Blocked.Remove key
    Call Persist: RenderGrid: Schedule 1
    Exit Sub
Failed:
    UnicodeMsgBox Err.Description, vbExclamation
End Sub

Private Function FindRow(ByVal key As String) As Long
    Dim row As Long
    For row = FIRST_ROW To Grid.Cells(Grid.Rows.Count, UUID_COL).End(xlUp).row
        If RowKey(row) = key Then FindRow = row: Exit Function
    Next
    FindRow = FIRST_ROW
End Function

Private Function ExcelDate(ByVal value As Variant) As Variant
    Dim text As String, result As Date
    text = TextValue(value)
    If Len(text) < 10 Then ExcelDate = Empty: Exit Function
    result = DateSerial(CInt(Left$(text, 4)), CInt(Mid$(text, 6, 2)), CInt(Mid$(text, 9, 2)))
    If Len(text) >= 19 Then result = result + TimeSerial(CInt(Mid$(text, 12, 2)), CInt(Mid$(text, 15, 2)), CInt(Mid$(text, 18, 2)))
    ExcelDate = CDbl(result)
End Function

Private Function ReceiptWhitespace(ByVal code As Long) As Boolean
    ' Python str.strip/splitlines whitespace, not only the ASCII space.
    Select Case code
        Case 9 To 13, 28 To 32, 133, 160, 5760, 8192 To 8202, 8232, 8233, 8239, 8287, 12288
            ReceiptWhitespace = True
    End Select
End Function

Private Function TrimReceiptLine(ByVal text As String) As String
    Dim first As Long, last As Long
    first = 1: last = Len(text)
    Do While first <= last
        If Not ReceiptWhitespace(AscW(Mid$(text, first, 1)) And &HFFFF&) Then Exit Do
        first = first + 1
    Loop
    Do While last >= first
        If Not ReceiptWhitespace(AscW(Mid$(text, last, 1)) And &HFFFF&) Then Exit Do
        last = last - 1
    Loop
    If first <= last Then TrimReceiptLine = Mid$(text, first, last - first + 1)
End Function

Private Function FirstReceiptLine(ByVal raw As String) As String
    Dim position As Long, first As Long, code As Long, boundary As Boolean, lineText As String
    first = 1
    For position = 1 To Len(raw) + 1
        boundary = (position > Len(raw))
        If Not boundary Then
            code = AscW(Mid$(raw, position, 1)) And &HFFFF&
            Select Case code
                Case 10 To 13, 28 To 30, 133, 8232, 8233: boundary = True
            End Select
        End If
        If boundary Then
            lineText = TrimReceiptLine(Mid$(raw, first, position - first))
            If Len(lineText) > 0 Then FirstReceiptLine = lineText: Exit Function
            first = position + 1
        End If
    Next
End Function

Private Function ReceiptCodePoint(ByVal text As String, ByVal position As Long, ByRef unitCount As Long) As Long
    Dim first As Long, second As Long
    unitCount = 1
    first = AscW(Mid$(text, position, 1)) And &HFFFF&
    ReceiptCodePoint = first
    If first >= &HD800& And first <= &HDBFF& And position < Len(text) Then
        second = AscW(Mid$(text, position + 1, 1)) And &HFFFF&
        If second >= &HDC00& And second <= &HDFFF& Then
            unitCount = 2
            ReceiptCodePoint = &H10000 + (first - &HD800&) * &H400& + second - &HDC00&
        End If
    End If
End Function

Private Function OperationSymbol(ByVal code As Long) As Boolean
    Static ready As Boolean, lowerBounds() As Long, upperBounds() As Long
    Dim encoded As String, pieces As Variant, pair As Variant, index As Long, low As Long, high As Long, middle As Long
    If Not ready Then
        ' Unicode 14.0 category S*, matching Python 3.11 in the backend.
        ' Generated from unicode.org/Public/14.0.0/ucd/UnicodeData.txt.
        encoded = _
            "24-24|2B-2B|3C-3E|5E-5E|60-60|7C-7C|7E-7E|A2-A6|A8-A9|AC-AC|AE-B1|B4-B4|B8-B8|D7-D7|F7-F7|2C2-2C5|2D2-2DF|2E5-2EB|2ED-2ED|2EF-2FF|375-375|384-385|3F6-3F6|482-482|58D-58F|606-608|" & _
            "60B-60B|60E-60F|6DE-6DE|6E9-6E9|6FD-6FE|7F6-7F6|7FE-7FF|888-888|9F2-9F3|9FA-9FB|AF1-AF1|B70-B70|BF3-BFA|C7F-C7F|D4F-D4F|D79-D79|E3F-E3F|F01-F03|F13-F13|F15-F17|F1A-F1F|F34-F34|" & _
            "F36-F36|F38-F38|FBE-FC5|FC7-FCC|FCE-FCF|FD5-FD8|109E-109F|1390-1399|166D-166D|17DB-17DB|1940-1940|19DE-19FF|1B61-1B6A|1B74-1B7C|1FBD-1FBD|1FBF-1FC1|1FCD-1FCF|1FDD-1FDF|1FED-1FEF|" & _
            "1FFD-1FFE|2044-2044|2052-2052|207A-207C|208A-208C|20A0-20C0|2100-2101|2103-2106|2108-2109|2114-2114|2116-2118|211E-2123|2125-2125|2127-2127|2129-2129|212E-212E|213A-213B|2140-2144|" & _
            "214A-214D|214F-214F|218A-218B|2190-2307|230C-2328|232B-2426|2440-244A|249C-24E9|2500-2767|2794-27C4|27C7-27E5|27F0-2982|2999-29D7|29DC-29FB|29FE-2B73|2B76-2B95|2B97-2BFF|2CE5-2CEA|" & _
            "2E50-2E51|2E80-2E99|2E9B-2EF3|2F00-2FD5|2FF0-2FFB|3004-3004|3012-3013|3020-3020|3036-3037|303E-303F|309B-309C|3190-3191|3196-319F|31C0-31E3|3200-321E|322A-3247|3250-3250|3260-327F|" & _
            "328A-32B0|32C0-33FF|4DC0-4DFF|A490-A4C6|A700-A716|A720-A721|A789-A78A|A828-A82B|A836-A839|AA77-AA79|AB5B-AB5B|AB6A-AB6B|FB29-FB29|FBB2-FBC2|FD40-FD4F|FDCF-FDCF|FDFC-FDFF|FE62-FE62|" & _
            "FE64-FE66|FE69-FE69|FF04-FF04|FF0B-FF0B|FF1C-FF1E|FF3E-FF3E|FF40-FF40|FF5C-FF5C|FF5E-FF5E|FFE0-FFE6|FFE8-FFEE|FFFC-FFFD|10137-1013F|10179-10189|1018C-1018E|10190-1019C|101A0-101A0|" & _
            "101D0-101FC|10877-10878|10AC8-10AC8|1173F-1173F|11FD5-11FF1|16B3C-16B3F|16B45-16B45|1BC9C-1BC9C|1CF50-1CFC3|1D000-1D0F5|1D100-1D126|1D129-1D164|1D16A-1D16C|1D183-1D184|1D18C-1D1A9|" & _
            "1D1AE-1D1EA|1D200-1D241|1D245-1D245|1D300-1D356|1D6C1-1D6C1|1D6DB-1D6DB|1D6FB-1D6FB|1D715-1D715|1D735-1D735|1D74F-1D74F|1D76F-1D76F|1D789-1D789|1D7A9-1D7A9|1D7C3-1D7C3|1D800-1D9FF|" & _
            "1DA37-1DA3A|1DA6D-1DA74|1DA76-1DA83|1DA85-1DA86|1E14F-1E14F|1E2FF-1E2FF|1ECAC-1ECAC|1ECB0-1ECB0|1ED2E-1ED2E|1EEF0-1EEF1|1F000-1F02B|1F030-1F093|1F0A0-1F0AE|1F0B1-1F0BF|1F0C1-1F0CF|" & _
            "1F0D1-1F0F5|1F10D-1F1AD|1F1E6-1F202|1F210-1F23B|1F240-1F248|1F250-1F251|1F260-1F265|1F300-1F6D7|1F6DD-1F6EC|1F6F0-1F6FC|1F700-1F773|1F780-1F7D8|1F7E0-1F7EB|1F7F0-1F7F0|1F800-1F80B|" & _
            "1F810-1F847|1F850-1F859|1F860-1F887|1F890-1F8AD|1F8B0-1F8B1|1F900-1FA53|1FA60-1FA6D|1FA70-1FA74|1FA78-1FA7C|1FA80-1FA86|1FA90-1FAAC|1FAB0-1FABA|1FAC0-1FAC5|1FAD0-1FAD9|1FAE0-1FAE7|" & _
            "1FAF0-1FAF6|1FB00-1FB92|1FB94-1FBCA"
        pieces = Split(encoded, "|")
        ReDim lowerBounds(0 To UBound(pieces)): ReDim upperBounds(0 To UBound(pieces))
        For index = 0 To UBound(pieces)
            pair = Split(pieces(index), "-")
            lowerBounds(index) = CLng("&H" & pair(0)): upperBounds(index) = CLng("&H" & pair(1))
        Next
        ready = True
    End If
    low = LBound(lowerBounds): high = UBound(lowerBounds)
    Do While low <= high
        middle = (low + high) \ 2
        If code < lowerBounds(middle) Then
            high = middle - 1
        ElseIf code > upperBounds(middle) Then
            low = middle + 1
        Else
            OperationSymbol = True: Exit Function
        End If
    Loop
End Function

Private Function OperationFromRaw(ByVal raw As String) As String
    Dim first As String, code As Long, unitCount As Long, firstUnits As Long, position As Long, count As Long
    first = FirstReceiptLine(raw)
    If Len(first) = 0 Then Exit Function
    position = 1
    Do While position <= Len(first)
        code = ReceiptCodePoint(first, position, unitCount)
        count = count + 1
        If count > 60 Then Exit Function
        position = position + unitCount
    Loop
    code = ReceiptCodePoint(first, 1, firstUnits)
    If Not OperationSymbol(code) Then
        If Mid$(first, firstUnits + 1, 1) <> Ui("\ufe0f") Then Exit Function
    End If
    ' These symbols introduce amounts, balances, merchants, cards or dates.
    Select Case code
        Case &H2796, &H2795, &H1F4B5, &H1F4B0, &H1F4CD, &H1F3EA, &H1F4B3, &H1F553, &H1F558, &H1F552, &H1F4C5
            Exit Function
    End Select
    OperationFromRaw = first
End Function

Private Function OperationDisplay(ByVal fields As Object, ByVal metadata As Object) As String
    Dim raw As String, kind As String, displayLabel As String, source As String, haystack As String, hint As Variant
    raw = TextValue(fields("raw_message"))
    OperationDisplay = OperationFromRaw(raw)
    If Len(OperationDisplay) > 0 Then Exit Function
    kind = TextValue(fields("transaction_type"))
    If InStr(1, UCase$(raw), Ui("\u0421\u0427\u0415\u0422 \u041f\u041e \u041a\u0410\u0420\u0422\u0415 \u0418\u0417\u041c\u0415\u041d"), vbBinaryCompare) > 0 Or _
       InStr(1, UCase$(raw), Ui("\u0421\u0427\u0401\u0422 \u041f\u041e \u041a\u0410\u0420\u0422\u0415 \u0418\u0417\u041c\u0415\u041d"), vbBinaryCompare) > 0 Then
        displayLabel = Ui("\u0418\u0437\u043c\u0435\u043d\u0435\u043d\u0438\u0435 \u0441\u0447\u0435\u0442\u0430")
    Else
        Select Case kind
            Case "CREDIT": displayLabel = Ui("\u041f\u043e\u043f\u043e\u043b\u043d\u0435\u043d\u0438\u0435")
            Case "CONVERSION": displayLabel = Ui("\u041a\u043e\u043d\u0432\u0435\u0440\u0441\u0438\u044f")
            Case "REVERSAL": displayLabel = Ui("\u041e\u0442\u043c\u0435\u043d\u0430")
            Case Else: displayLabel = Ui("\u0421\u043f\u0438\u0441\u0430\u043d\u0438\u0435")
        End Select
    End If
    source = TextValue(StateFieldValue(fields, "excel_bank"))
    If source <> "CardXabar" And source <> "HUMO Card" Then
        source = ""
        If metadata.Exists("source_display") Then source = TextValue(metadata("source_display"))
    End If
    If InStr(1, UCase$(source), "HUMO", vbBinaryCompare) = 0 Then
        OperationDisplay = Ui("\ud83d\udd34 ") & displayLabel
        Exit Function
    End If
    If InStr(1, LCase$(displayLabel), Ui("\u0438\u0437\u043c\u0435\u043d\u0435\u043d\u0438\u0435"), vbBinaryCompare) > 0 Then
        OperationDisplay = Ui("\u2139\ufe0f ") & displayLabel
        Exit Function
    End If
    haystack = LCase$(TextValue(fields("operator_raw")) & " " & raw)
    If kind = "DEBIT" Then
        For Each hint In Array(Ui("\u0441\u043d\u044f\u0442\u0438\u0435"), Ui("\u043e\u0431\u043d\u0430\u043b"), "atm", Ui("\u0431\u0430\u043d\u043a\u043e\u043c\u0430\u0442"), "bxo")
            If InStr(1, haystack, CStr(hint), vbBinaryCompare) > 0 Then
                OperationDisplay = Ui("\ud83c\udfe7 ") & displayLabel
                Exit Function
            End If
        Next
    End If
    Select Case kind
        Case "DEBIT": OperationDisplay = Ui("\ud83d\udcb8 ")
        Case "CREDIT": OperationDisplay = Ui("\ud83c\udf89 ")
        Case "CONVERSION": OperationDisplay = Ui("\ud83d\udcb1 ")
        Case "REVERSAL": OperationDisplay = Ui("\u21a9\ufe0f ")
        Case Else: OperationDisplay = Ui("\u2139\ufe0f ")
    End Select
    OperationDisplay = OperationDisplay & displayLabel
End Function

Private Function DisplayValue(ByVal field As String, ByVal value As Variant) As Variant
    If IsNull(value) Then DisplayValue = Empty: Exit Function
    Select Case field
        Case "transaction_date", "annotation_period": DisplayValue = ExcelDate(value)
        Case "amount": DisplayValue = Abs(Val(CStr(value)))
        Case "balance_after": DisplayValue = Val(CStr(value))
        Case "is_p2p": If value Then DisplayValue = 1 Else DisplayValue = 0
        Case Else
            If Len(CStr(value)) > 32767 Then
                DisplayValue = Left$(CStr(value), 32710) & " [TRUNCATED DISPLAY; full text retained on server]"
            Else
                DisplayValue = CStr(value)
            End If
    End Select
End Function

Private Function SameGridValue(ByVal actual As Variant, ByVal expected As Variant) As Boolean
    Dim leftNumber As Double, rightNumber As Double, comparisonScale As Double
    If IsError(actual) Or IsError(expected) Then Exit Function
    If IsNull(expected) Or IsEmpty(expected) Then
        SameGridValue = IsNull(actual) Or IsEmpty(actual)
        If VarType(actual) = vbString Then SameGridValue = (Len(actual) = 0)
    ElseIf VarType(expected) = vbString Then
        If Len(expected) = 0 And (IsNull(actual) Or IsEmpty(actual)) Then SameGridValue = True: Exit Function
        If VarType(actual) = vbString Then SameGridValue = (StrComp(actual, expected, vbBinaryCompare) = 0)
    ElseIf VarType(expected) = vbBoolean Then
        If VarType(actual) = vbBoolean Then SameGridValue = (actual = expected)
    ElseIf IsNumeric(expected) Then
        ' Excel stores at most 15 significant decimal digits. Permit only that
        ' rounding, never text/number, blank/zero or Boolean/zero coercions.
        If IsNull(actual) Or IsEmpty(actual) Then Exit Function
        If VarType(actual) = vbString Or VarType(actual) = vbBoolean Then Exit Function
        If Not IsNumeric(actual) Then Exit Function
        leftNumber = CDbl(actual): rightNumber = CDbl(expected)
        If (leftNumber = 0) Xor (rightNumber = 0) Then Exit Function
        comparisonScale = Abs(rightNumber)
        If comparisonScale < 1 Then comparisonScale = 1
        SameGridValue = (Abs(leftNumber - rightNumber) <= comparisonScale * 0.000000000000001)
    End If
End Function

Private Sub CopyStyleColor(ByVal source As Object, ByVal target As Object)
    Dim theme As Variant
    target.Color = source.Color
    If source.ColorIndex = xlColorIndexAutomatic Then target.ColorIndex = xlColorIndexAutomatic
    ' ThemeColor raises for ordinary RGB/automatic colors. In that case the
    ' resolved RGB above already preserves the visible color exactly.
    On Error Resume Next
    Err.Clear
    theme = source.ThemeColor
    If Err.Number = 0 Then target.ThemeColor = theme
    On Error GoTo 0
    target.TintAndShade = source.TintAndShade
End Sub

Private Function IsCompactTextColumn(ByVal column As Long) As Boolean
    ' Long labels remain inside their own cells. Fixed row height shows the
    ' first line at the original font size; the complete value stays editable.
    Select Case column
        Case 19, 20, 21, 23, 37, 38, 40, 42, 43, 44, 45, 46, 47, 48, 50
            IsCompactTextColumn = True
    End Select
End Function

Private Sub ClampGridRows(ByVal target As Range, ByVal height As Double)
    Dim col As Long, part As Range, expectedWrap As Boolean, savedIndent As Long
    For col = 15 To 53
        Set part = target.Cells(1, col - 14)
        expectedWrap = IsCompactTextColumn(col)
        If part.WrapText <> expectedWrap Then
            ' A mixed-alignment range assignment resets date indentation.
            ' Change individual cells and retain their existing indentation.
            savedIndent = part.IndentLevel
            part.WrapText = expectedWrap
            If part.IndentLevel <> savedIndent Then part.IndentLevel = savedIndent
        End If
        If expectedWrap Then
            If part.VerticalAlignment <> xlTop Then part.VerticalAlignment = xlTop
            If part.ShrinkToFit Then part.ShrinkToFit = False
        End If
    Next
    ' Explicitly fix even default-height rows: later wrapping must not autofit.
    target.RowHeight = height
End Sub

Private Sub DefineGridStyle(ByVal formatting As Style, ByVal source As Range)
    Dim edge As Variant, patternTheme As Variant
    formatting.IncludeAlignment = True: formatting.IncludeBorder = True
    formatting.IncludeFont = True: formatting.IncludeNumber = True: formatting.IncludePatterns = True
    formatting.NumberFormat = source.NumberFormat
    formatting.HorizontalAlignment = source.HorizontalAlignment
    formatting.VerticalAlignment = source.VerticalAlignment
    formatting.Orientation = source.Orientation: formatting.IndentLevel = source.IndentLevel
    formatting.ReadingOrder = source.ReadingOrder: formatting.ShrinkToFit = source.ShrinkToFit
    formatting.WrapText = IsCompactTextColumn(source.Column)
    If IsCompactTextColumn(source.Column) Then
        formatting.VerticalAlignment = xlTop
        formatting.ShrinkToFit = False
    End If
    With formatting.Font
        .Name = source.Font.Name: .Size = source.Font.Size
        .Bold = source.Font.Bold: .Italic = source.Font.Italic
        .Underline = source.Font.Underline: .Strikethrough = source.Font.Strikethrough
        .Subscript = source.Font.Subscript: .Superscript = source.Font.Superscript
        .OutlineFont = source.Font.OutlineFont: .Shadow = source.Font.Shadow
        .ThemeFont = source.Font.ThemeFont
    End With
    CopyStyleColor source.Font, formatting.Font
    CopyStyleColor source.Interior, formatting.Interior
    formatting.Interior.PatternColor = source.Interior.PatternColor
    On Error Resume Next
    Err.Clear
    patternTheme = source.Interior.PatternThemeColor
    If Err.Number = 0 Then formatting.Interior.PatternThemeColor = patternTheme
    On Error GoTo 0
    formatting.Interior.PatternTintAndShade = source.Interior.PatternTintAndShade
    ' Color assignments can turn a no-fill cell into a solid fill.
    formatting.Interior.Pattern = source.Interior.Pattern
    ' Style.Borders uses xlLeft/xlTop/etc.; Range.Borders uses xlEdgeLeft/etc.
    For Each edge In Array(Array(xlDiagonalDown, xlDiagonalDown), Array(xlDiagonalUp, xlDiagonalUp), _
                           Array(xlEdgeLeft, xlLeft), Array(xlEdgeTop, xlTop), _
                           Array(xlEdgeBottom, xlBottom), Array(xlEdgeRight, xlRight))
        If source.Borders(edge(0)).LineStyle = xlLineStyleNone Then
            formatting.Borders(edge(1)).LineStyle = xlLineStyleNone
        Else
            formatting.Borders(edge(1)).Weight = source.Borders(edge(0)).Weight
            CopyStyleColor source.Borders(edge(0)), formatting.Borders(edge(1))
            formatting.Borders(edge(1)).LineStyle = source.Borders(edge(0)).LineStyle
        End If
    Next
    formatting.IncludeProtection = False
End Sub

Private Function RestoreStyleSheets(ByVal saved As Collection) As String
    Dim item As Variant, sheet As Worksheet
    If saved Is Nothing Then Exit Function
    On Error Resume Next
    For Each item In saved
        Set sheet = item("sheet")
        Err.Clear
        sheet.Protect Password:=LAYOUT_PASSWORD, DrawingObjects:=item("drawing"), Contents:=item("contents"), _
            Scenarios:=item("scenarios"), UserInterfaceOnly:=item("ui_only"), _
            AllowFormattingCells:=item("format_cells"), AllowFormattingColumns:=item("format_columns"), _
            AllowFormattingRows:=item("format_rows"), AllowInsertingColumns:=item("insert_columns"), _
            AllowInsertingRows:=item("insert_rows"), AllowInsertingHyperlinks:=item("insert_links"), _
            AllowDeletingColumns:=item("delete_columns"), AllowDeletingRows:=item("delete_rows"), _
            AllowSorting:=item("sort"), AllowFiltering:=item("filter"), AllowUsingPivotTables:=item("pivots")
        If Err.Number <> 0 And Len(RestoreStyleSheets) = 0 Then RestoreStyleSheets = Err.Description
        sheet.EnableSelection = item("selection")
    Next
    On Error GoTo 0
End Function

Private Function UnprotectStyleSheets() As Collection
    Dim saved As New Collection, item As Object, sheet As Worksheet, index As Long, message As String, restoreError As String
    On Error GoTo Failed
    ' Excel refuses to define even a new, unused style while any worksheet is
    ' protected. Capture every permission before changing anything, then put
    ' the original protection back even if style definition fails.
    For index = 1 To ThisWorkbook.Worksheets.Count
        Set sheet = ThisWorkbook.Worksheets.Item(index)
        If sheet.ProtectContents Or sheet.ProtectDrawingObjects Or sheet.ProtectScenarios Then
            Set item = Dict(): item.Add "sheet", sheet
            item.Add "drawing", sheet.ProtectDrawingObjects: item.Add "contents", sheet.ProtectContents
            item.Add "scenarios", sheet.ProtectScenarios: item.Add "ui_only", sheet.ProtectionMode
            item.Add "selection", sheet.EnableSelection
            With sheet.Protection
                item.Add "format_cells", .AllowFormattingCells: item.Add "format_columns", .AllowFormattingColumns
                item.Add "format_rows", .AllowFormattingRows: item.Add "insert_columns", .AllowInsertingColumns
                item.Add "insert_rows", .AllowInsertingRows: item.Add "insert_links", .AllowInsertingHyperlinks
                item.Add "delete_columns", .AllowDeletingColumns: item.Add "delete_rows", .AllowDeletingRows
                item.Add "sort", .AllowSorting: item.Add "filter", .AllowFiltering: item.Add "pivots", .AllowUsingPivotTables
            End With
            saved.Add item
        End If
    Next
    For Each item In saved
        Set sheet = item("sheet"): sheet.Unprotect LAYOUT_PASSWORD
    Next
    Set UnprotectStyleSheets = saved
    Exit Function
Failed:
    message = Err.Description
    restoreError = RestoreStyleSheets(saved)
    If Len(restoreError) > 0 Then message = message & "; protection: " & restoreError
    Err.Raise 5, , message
End Function

Private Sub EnsureGridStyles()
    Dim col As Long, name As String, formatting As Style, created As Boolean, message As String
    Dim protectedSheets As Collection, restoreError As String
    If mGridStylesReady Then Exit Sub
    On Error GoTo Failed
    For col = 15 To 53
        name = "_TBSsync_20260915_operation_c" & col
        Set formatting = Nothing: created = False
        On Error Resume Next
        Set formatting = ThisWorkbook.Styles(name)
        On Error GoTo Failed
        If formatting Is Nothing Then
            If protectedSheets Is Nothing Then Set protectedSheets = UnprotectStyleSheets()
            ' Styles.Add(BasedOn:=hiddenRange) fails in native Excel. Define a
            ' blank style directly; no activation or clipboard is necessary.
            Set formatting = ThisWorkbook.Styles.Add(Name:=name): created = True
            DefineGridStyle formatting, ThisWorkbook.Worksheets("_style").Cells(FIRST_ROW, col)
            created = False
        End If
    Next
    restoreError = RestoreStyleSheets(protectedSheets)
    If Len(restoreError) > 0 Then Err.Raise 5, , "Could not restore worksheet protection: " & restoreError
    mGridStylesReady = True
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    If created Then formatting.Delete
    On Error GoTo 0
    restoreError = RestoreStyleSheets(protectedSheets)
    If Len(restoreError) > 0 Then message = message & "; protection: " & restoreError
    Err.Raise 5, , "Cannot prepare column " & col & " formatting: " & message
End Sub

Public Sub ViewEnsureColumnStyles(ByVal styleSheet As String, ByVal prefix As String, ByVal lastColumn As Long)
    Dim col As Long, name As String, formatting As Style, source As Range, created As Boolean
    Dim protectedSheets As Collection, view As Object, message As String, restoreError As String
    On Error GoTo Failed
    Set view = BeginViewUpdate()
    For col = 15 To lastColumn
        name = prefix & col
        Set formatting = Nothing: created = False
        On Error Resume Next
        Set formatting = ThisWorkbook.Styles(name)
        On Error GoTo Failed
        If formatting Is Nothing Then
            If protectedSheets Is Nothing Then Set protectedSheets = UnprotectStyleSheets()
            Set formatting = ThisWorkbook.Styles.Add(Name:=name): created = True
            Set source = ThisWorkbook.Worksheets(styleSheet).Cells(FIRST_ROW, col)
            DefineGridStyle formatting, source
            ' Bank prototypes use physical columns different from the hidden
            ' core. Their own exact alignment policy takes precedence.
            formatting.WrapText = source.WrapText
            formatting.VerticalAlignment = source.VerticalAlignment
            formatting.ShrinkToFit = source.ShrinkToFit
            formatting.IncludeProtection = False
            created = False
        End If
    Next
    restoreError = RestoreStyleSheets(protectedSheets)
    If Len(restoreError) > 0 Then Err.Raise 5, , "Could not restore worksheet protection: " & restoreError
    EndViewUpdate view
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    If created Then formatting.Delete
    On Error GoTo 0
    restoreError = RestoreStyleSheets(protectedSheets)
    EndViewUpdate view
    If Len(restoreError) > 0 Then message = message & "; protection: " & restoreError
    Err.Raise 5, , "Cannot prepare bank column " & col & " formatting: " & message
End Sub

Private Sub WriteGridChanges(ByRef desired() As Variant, ByVal last As Long, ByVal inputRow As Long, ByVal positions As Object)
    Dim current As Variant, row As Long, col As Long, firstCol As Long, endCol As Long, i As Long, absoluteRow As Long
    Dim changed As Object, hidden As Object, formulas As Object, key As Variant, formatRows As Range, part As Range, view As Object
    Dim formulaCells As Range, formulaCell As Range
    Dim block() As Variant, message As String, wasSaved As Boolean, height As Double, inputLabel As String
    Dim formulaState As Variant, hasFormulas As Boolean
    Set changed = Dict(): Set hidden = Dict(): Set formulas = Dict()
    current = Grid.Range("O5:BA" & last).Value2
    ' In Excel, a failing SpecialCells("no cells") call dirties the workbook.
    ' HasFormula is read-only and avoids that side effect on ordinary polls.
    formulaState = Grid.Range("O5:BA" & last).HasFormula
    hasFormulas = IsNull(formulaState)
    If Not hasFormulas Then hasFormulas = CBool(formulaState)
    If hasFormulas Then
        On Error Resume Next
        Set formulaCells = Grid.Range("O5:BA" & last).SpecialCells(xlCellTypeFormulas)
        On Error GoTo 0
    End If
    If Not formulaCells Is Nothing Then
        For Each formulaCell In formulaCells.Cells
            formulas(CStr(formulaCell.row - FIRST_ROW + 1) & ":" & CStr(formulaCell.Column - 14)) = True
        Next
    End If
    inputLabel = CStr(desired(inputRow - FIRST_ROW + 1, 36))
    For row = 1 To UBound(desired, 1)
        absoluteRow = FIRST_ROW + row - 1
        For col = 1 To 39
            If formulas.Exists(CStr(row) & ":" & CStr(col)) Or Not SameGridValue(current(row, col), desired(row, col)) Then
                changed(CStr(row)) = True
                Exit For
            End If
        Next
        If changed.Exists(CStr(row)) Then
            hidden(CStr(row)) = CBool(Grid.Rows(absoluteRow).hidden)
            If (Len(TextValue(current(row, 38))) = 0 And Len(TextValue(desired(row, 38))) > 0) Or _
               (absoluteRow = inputRow And TextValue(current(row, 36)) <> inputLabel) Then
                Set part = Grid.Range("O" & absoluteRow & ":BA" & absoluteRow)
                If formatRows Is Nothing Then Set formatRows = part Else Set formatRows = Union(formatRows, part)
            End If
        End If
    Next
    If changed.Count = 0 And mInputsReady Then Exit Sub
    wasSaved = ThisWorkbook.Saved
    On Error GoTo Failed
    Set view = BeginViewUpdate()
    If changed.Count > 0 Then
        Grid.Unprotect LAYOUT_PASSWORD
        If Not formatRows Is Nothing Then
            EnsureGridStyles
            For col = 15 To 53
                Intersect(formatRows, Grid.Columns(col)).Style = "_TBSsync_20260915_operation_c" & col
            Next
        End If
        height = ThisWorkbook.Worksheets("_style").Rows(FIRST_ROW).RowHeight
        For Each key In changed.Keys
            row = CLng(key): absoluteRow = FIRST_ROW + row - 1: col = 1
            Do While col <= 39
                If Not formulas.Exists(CStr(row) & ":" & CStr(col)) And SameGridValue(current(row, col), desired(row, col)) Then
                    col = col + 1
                Else
                    firstCol = col: endCol = col
                    Do While endCol < 39
                        If Not formulas.Exists(CStr(row) & ":" & CStr(endCol + 1)) And SameGridValue(current(row, endCol + 1), desired(row, endCol + 1)) Then Exit Do
                        endCol = endCol + 1
                    Loop
                    ReDim block(1 To 1, 1 To endCol - firstCol + 1)
                    For i = firstCol To endCol: block(1, i - firstCol + 1) = desired(row, i): Next
                    Grid.Range(Grid.Cells(absoluteRow, firstCol + 14), Grid.Cells(absoluteRow, endCol + 14)).Value2 = block
                    col = endCol + 1
                End If
            Loop
            ClampGridRows Grid.Range("O" & absoluteRow & ":BA" & absoluteRow), height
            ' Row height must not reveal rows hidden by the user's filter.
            If hidden(key) Then Grid.Rows(absoluteRow).hidden = True
        Next
    End If
    ProtectInputs
    If changed.Count > 0 Then Grid.Calculate
    EndViewUpdate view, positions
    If changed.Count = 0 And wasSaved Then ThisWorkbook.Saved = True
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    ProtectInputs
    EndViewUpdate view
    On Error GoTo 0
    Err.Raise 5, , message
End Sub

Private Sub RenderGrid()
    Dim order As New Collection, displayed As New Collection, seen As Object, row As Long, key As Variant, keys() As String
    Dim fields As Object, record As Object, values() As Variant, col As Long, field As String, last As Long
    Dim inputRow As Long, clearLast As Long, positions As Object, structural As Boolean, message As String
    If Not mReady Then Exit Sub
    If Not mHasSnapshot Then ProtectInputs: Exit Sub
    On Error GoTo Failed
    Set seen = Dict(): last = Grid.Cells(Grid.Rows.Count, UUID_COL).End(xlUp).row
    clearLast = Grid.Cells(Grid.Rows.Count, 50).End(xlUp).row
    If clearLast < last Then clearLast = last
    For row = FIRST_ROW To last
        key = DisplayKey(RowKey(row))
        displayed.Add key
        If Records.Exists(key) And Not seen.Exists(key) Then order.Add key: seen(key) = True
    Next
    For Each key In Records.Keys
        If Not seen.Exists(key) Then order.Add key
    Next
    structural = (displayed.Count <> order.Count)
    If Not structural Then
        For row = 1 To order.Count
            If CStr(displayed(row)) <> CStr(order(row)) Then structural = True: Exit For
        Next
    End If
    If structural Then
        If Not ActiveWorkbook Is ThisWorkbook Then
            ' Moving rows in an inactive window would move receipts under its
            ' selection. Keep its view untouched until Workbook_Activate.
            If Not mRenderDeferred Then
                mRenderDeferred = True: mInputsReady = False
                ProtectInputs
                BankViews.SetEditable False
            End If
            Exit Sub
        End If
    End If
    If mRenderDeferred Then mRenderDeferred = False: mInputsReady = False
    inputRow = FIRST_ROW + order.Count
    If clearLast < inputRow Then clearLast = inputRow
    ReDim values(1 To clearLast - FIRST_ROW + 1, 1 To 39)
    Set positions = Dict()
    If order.Count > 0 Then
        For row = 1 To order.Count
            key = order(row): Set record = Records.Item(key)
            ' Rendering only reads fields. Avoid JSON-copying every unchanged
            ' receipt (including its original text) on every polling tick.
            If Intents.Exists(key) Then Set fields = EffectiveFields(CStr(key)) Else Set fields = record("fields")
            positions(CStr(key)) = FIRST_ROW + row - 1
            values(row, 1) = row: values(row, 2) = row
            For col = 17 To 44
                field = ColumnField(col)
                If Len(field) > 0 Then values(row, col - 14) = DisplayValue(field, fields(field))
            Next
            values(row, 6) = OperationDisplay(fields, record("metadata"))
            values(row, 4) = values(row, 3)
            If record("metadata").Exists("source_display") Then values(row, 5) = record("metadata")("source_display")
            If fields("transaction_type") = "DEBIT" Then values(row, 10) = "-" Else values(row, 10) = "+"
            values(row, 11) = values(row, 12)
            values(row, 18) = values(row, 19)
            If record("metadata").Exists("source_type") Then values(row, 31) = record("metadata")("source_type")
            If record("metadata").Exists("source_chat_id") Then values(row, 32) = TextValue(record("metadata")("source_chat_id"))
            If record("metadata").Exists("source_message_id") Then values(row, 33) = TextValue(record("metadata")("source_message_id"))
            If record("metadata").Exists("parsing_method") Then values(row, 34) = record("metadata")("parsing_method")
            If record("metadata").Exists("parsing_confidence") Then values(row, 35) = record("metadata")("parsing_confidence")
            values(row, 36) = Ui("\u0421\u043e\u0445\u0440\u0430\u043d\u0435\u043d\u043e \u00b7 \u0434\u0435\u0439\u0441\u0442\u0432\u0438\u044f")
            If Intents.Exists(key) Then
                Select Case Intents.Item(key)("kind")
                    Case "draft": values(row, 36) = Ui("\u0427\u0435\u0440\u043d\u043e\u0432\u0438\u043a: \u0437\u0430\u043f\u043e\u043b\u043d\u0438\u0442\u0435 \u043f\u043e\u043b\u044f")
                    Case "delete": values(row, 36) = Ui("\u041e\u0436\u0438\u0434\u0430\u0435\u0442 \u0443\u0434\u0430\u043b\u0435\u043d\u0438\u044f")
                    Case Else: values(row, 36) = Ui("\u041e\u0436\u0438\u0434\u0430\u0435\u0442 \u043e\u0442\u043f\u0440\u0430\u0432\u043a\u0438")
                End Select
            End If
            If Blocked.Exists(key) Then values(row, 36) = Ui("\u0422\u0440\u0435\u0431\u0443\u0435\u0442 \u0432\u043d\u0438\u043c\u0430\u043d\u0438\u044f \u00b7 \u0434\u0435\u0439\u0441\u0442\u0432\u0438\u044f")
            If Len(TextValue(fields("raw_message"))) > 32767 Then values(row, 36) = CStr(values(row, 36)) & "; text display truncated"
            values(row, 37) = CStr(record("id")): values(row, 38) = CStr(record("uuid")): values(row, 39) = CStr(record("version"))
        Next
    End If
    values(inputRow - FIRST_ROW + 1, 36) = Ui("\u041d\u043e\u0432\u044b\u0439 \u0447\u0435\u043a: \u0437\u0430\u043f\u043e\u043b\u043d\u0438\u0442\u0435 \u0441\u0442\u0440\u043e\u043a\u0443")
    If Not mRowAliases Is Nothing Then
        For Each key In mRowAliases.Keys
            If positions.Exists(DisplayKey(CStr(key))) Then positions(CStr(key)) = positions(DisplayKey(CStr(key)))
        Next
    End If
    If mState.Exists("view_aliases") Then
        For Each key In mState("view_aliases").Keys
            If positions.Exists(DisplayKey(CStr(key))) Then positions(CStr(key)) = positions(DisplayKey(CStr(key)))
        Next
    End If
    WriteGridChanges values, clearLast, inputRow, positions
    BankViews.Render
    Set mRowAliases = Nothing
    Exit Sub
Failed:
    message = Err.Description
    mRenderDeferred = True: mInputsReady = False
    On Error Resume Next
    ProtectInputs
    BankViews.SetEditable False
    On Error GoTo 0
    Err.Raise 5, , "Cannot display synchronization state: " & message
End Sub

Private Sub ProtectInputs()
    Dim row As Long, col As Long, key As String, last As Long, editable As Range, part As Range
    Dim view As Object, message As String, wasSaved As Boolean
    wasSaved = ThisWorkbook.Saved
    On Error GoTo Failed
    Set view = BeginViewUpdate()
    Grid.Unprotect LAYOUT_PASSWORD
    Grid.Cells.Locked = True
    last = Grid.Cells(Grid.Rows.Count, UUID_COL).End(xlUp).row + 1
    If last < FIRST_ROW Then last = FIRST_ROW
    If mHasSnapshot And Not mRenderDeferred Then
        For col = 17 To 44
            If Len(ColumnField(col)) > 0 Then
                Set part = Grid.Range(Grid.Cells(FIRST_ROW, col), Grid.Cells(last, col))
                If editable Is Nothing Then Set editable = part Else Set editable = Union(editable, part)
            End If
        Next
        editable.Locked = False
        For row = FIRST_ROW To last
            key = RowKey(row)
            If Intents.Exists(key) Then
                If Intents.Item(key)("kind") = "delete" Then Grid.Range("O" & row & ":BA" & row).Locked = True
            End If
        Next
    End If
    If IsNull(Grid.Columns("AY:BA").Hidden) Then
        Grid.Columns("AY:BA").Hidden = True
    ElseIf Not Grid.Columns("AY:BA").Hidden Then
        Grid.Columns("AY:BA").Hidden = True
    End If
    Grid.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True, AllowFiltering:=True
    mInputsReady = True
    EndViewUpdate view
    If wasSaved Then ThisWorkbook.Saved = True
    Exit Sub
Failed:
    message = Err.Description
    EndViewUpdate view
    Err.Raise 5, , message
End Sub

Public Sub SortByDate()
    Dim last As Long, view As Object, message As String
    If Not mReady Or mBusy Then Exit Sub
    If BankViews.Enabled() Then BankViews.SortByDate: Exit Sub
    On Error GoTo Failed
    Set view = BeginViewUpdate()
    last = Grid.Cells(Grid.Rows.Count, UUID_COL).End(xlUp).row
    If last >= FIRST_ROW Then
        Grid.Unprotect LAYOUT_PASSWORD
        Grid.Range("O5:BA" & last).Sort Key1:=Grid.Range("Q5"), Order1:=xlAscending, Header:=xlNo
        ProtectInputs
    End If
    EndViewUpdate view
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    ProtectInputs
    EndViewUpdate view
    UnicodeMsgBox message, vbExclamation
End Sub

Private Sub UpdateReferences(ByVal references As Object)
    Dim sheet As Worksheet, item As Variant, row As Long, col As Long, last As Long, view As Object, message As String
    Dim desired() As Variant, current As Variant, different As Boolean
    Set sheet = ThisWorkbook.Worksheets("_refs")
    last = sheet.Cells(sheet.Rows.Count, 1).End(xlUp).row
    If sheet.Cells(sheet.Rows.Count, 3).End(xlUp).row > last Then last = sheet.Cells(sheet.Rows.Count, 3).End(xlUp).row
    If references("persons").Count + 1 > last Then last = references("persons").Count + 1
    If references("articles").Count + 1 > last Then last = references("articles").Count + 1
    If last < 2 Then last = 2
    ReDim desired(1 To last - 1, 1 To 4)
    row = 1
    For Each item In references("persons")
        desired(row, 1) = TextValue(item("code")): desired(row, 2) = TextValue(item("label")): row = row + 1
    Next
    row = 1
    For Each item In references("articles")
        desired(row, 3) = TextValue(item("code")): desired(row, 4) = TextValue(item("label")): row = row + 1
    Next
    current = sheet.Range("A2:D" & last).Value2
    For row = 1 To last - 1
        For col = 1 To 4
            If Not SameGridValue(current(row, col), desired(row, col)) Then different = True: Exit For
        Next
        If different Then Exit For
    Next
    If Not different Then Exit Sub
    On Error GoTo Failed
    Set view = BeginViewUpdate()
    sheet.Unprotect LAYOUT_PASSWORD
    sheet.Range("A2:D" & last).Value2 = desired
    sheet.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True
    EndViewUpdate view
    Exit Sub
Failed:
    message = Err.Description
    On Error Resume Next
    sheet.Protect Password:=LAYOUT_PASSWORD, UserInterfaceOnly:=True
    EndViewUpdate view
    On Error GoTo 0
    Err.Raise 5, , message
End Sub

Public Function SelfTestOperations() As String
    Dim fields As Object, metadata As Object, headline As String, marker As Variant, originalRaw As String
    Set fields = EmptyFields(): Set metadata = Dict()
    metadata.Add "source_display", "CardXabar"
    headline = Ui("\ud83d\udd34 Platezh")
    fields("raw_message") = vbCrLf & Ui("\u00a0") & headline & "  " & vbCrLf & Ui("\ud83d\udccd Merchant")
    If OperationDisplay(fields, metadata) <> headline Then Err.Raise 5, , "Original bank headline was changed"
    fields("raw_message") = Ui("\u2139\ufe0f Notice") & vbLf & "details"
    If OperationDisplay(fields, metadata) <> Ui("\u2139\ufe0f Notice") Then Err.Raise 5, , "Variation-selector headline was rejected"
    If OperationFromRaw(Ui("\u2139 Notice")) <> "" Then Err.Raise 5, , "Letterlike symbol without presentation selector was accepted"
    If OperationFromRaw("[Photo]" & vbLf & headline) <> "" Then Err.Raise 5, , "OCR preamble was treated as a bank headline"
    If OperationFromRaw("Transfer through terminal") <> "" Then Err.Raise 5, , "Plain text was treated as a bank headline"
    If OperationFromRaw("=literal header") <> "=literal header" Then Err.Raise 5, , "Mathematical symbol headline was rejected"
    headline = Ui("\ud83d\udd34") & String$(59, "x")
    If OperationFromRaw(headline) <> headline Or OperationFromRaw(headline & "x") <> "" Then Err.Raise 5, , "Headline length did not count Unicode code points"
    For Each marker In Array("\u2796", "\u2795", "\ud83d\udcb5", "\ud83d\udcb0", "\ud83d\udccd", "\ud83c\udfea", "\ud83d\udcb3", "\ud83d\udd53", "\ud83d\udd58", "\ud83d\udd52", "\ud83d\udcc5")
        If OperationFromRaw(Ui(CStr(marker)) & " field") <> "" Then Err.Raise 5, , "Receipt field marker became an operation"
    Next
    fields("raw_message") = "[Photo]"
    fields("operator_raw") = "ATM"
    If OperationDisplay(fields, metadata) <> Ui("\ud83d\udd34 \u0421\u043f\u0438\u0441\u0430\u043d\u0438\u0435") Then Err.Raise 5, , "CardXabar fallback changed"
    metadata("source_display") = "HUMO Card"
    If OperationDisplay(fields, metadata) <> Ui("\ud83c\udfe7 \u0421\u043f\u0438\u0441\u0430\u043d\u0438\u0435") Then Err.Raise 5, , "HUMO withdrawal fallback changed"
    fields("operator_raw") = "shop"
    If OperationDisplay(fields, metadata) <> Ui("\ud83d\udcb8 \u0421\u043f\u0438\u0441\u0430\u043d\u0438\u0435") Then Err.Raise 5, , "HUMO debit fallback changed"
    metadata("source_display") = "Manual": fields("excel_bank") = "HUMO Card"
    headline = OperationDisplay(fields, metadata)
    If headline <> Ui("\ud83d\udcb8 \u0421\u043f\u0438\u0441\u0430\u043d\u0438\u0435") Then Err.Raise 5, , "Manual HUMO receipt used CardXabar fallback"
    If ReadOperationKind(headline, fields, metadata) <> "DEBIT" Then Err.Raise 5, , "Manual bank fallback could not be copied back"
    fields("excel_bank") = "CardXabar": metadata("source_display") = "HUMO Card"
    If OperationDisplay(fields, metadata) <> Ui("\ud83d\udd34 \u0421\u043f\u0438\u0441\u0430\u043d\u0438\u0435") Then Err.Raise 5, , "Explicit bank assignment did not override metadata fallback"
    fields("excel_bank") = Null
    fields("transaction_type") = "CREDIT"
    If OperationDisplay(fields, metadata) <> Ui("\ud83c\udf89 \u041f\u043e\u043f\u043e\u043b\u043d\u0435\u043d\u0438\u0435") Then Err.Raise 5, , "HUMO credit fallback changed"
    fields("transaction_type") = "CONVERSION"
    If OperationDisplay(fields, metadata) <> Ui("\ud83d\udcb1 \u041a\u043e\u043d\u0432\u0435\u0440\u0441\u0438\u044f") Then Err.Raise 5, , "Conversion fallback changed"
    fields("transaction_type") = "REVERSAL"
    If OperationDisplay(fields, metadata) <> Ui("\u21a9\ufe0f \u041e\u0442\u043c\u0435\u043d\u0430") Then Err.Raise 5, , "Reversal fallback changed"
    fields("raw_message") = Ui("\u0421\u0447\u0451\u0442 \u043f\u043e \u043a\u0430\u0440\u0442\u0435 \u0438\u0437\u043c\u0435\u043d\u0435\u043d")
    If OperationDisplay(fields, metadata) <> Ui("\u2139\ufe0f \u0418\u0437\u043c\u0435\u043d\u0435\u043d\u0438\u0435 \u0441\u0447\u0435\u0442\u0430") Then Err.Raise 5, , "Balance-change notice fallback changed"
    fields("transaction_type") = "DEBIT": fields("raw_message") = Ui("\ud83d\udd34 Platezh")
    originalRaw = CStr(fields("raw_message")): headline = OperationDisplay(fields, metadata)
    If ReadOperationKind(headline, fields, metadata) <> "DEBIT" Then Err.Raise 5, , "Copying a displayed headline changed its canonical type"
    If ReadOperationKind("credit", fields, metadata) <> "CREDIT" Then Err.Raise 5, , "Explicit canonical type was not accepted"
    If fields("raw_message") <> originalRaw Or fields("transaction_type") <> "DEBIT" Then Err.Raise 5, , "Type interpretation rewrote original data"
    If AmountForOperation(12.34, headline, "DEBIT") <> "-12.34" Then Err.Raise 5, , "Displayed headline reversed debit amount sign"
    If AmountForOperation(12.34, headline, "CREDIT") <> "12.34" Then Err.Raise 5, , "Displayed headline reversed credit amount sign"
    If AmountForOperation(12.34, "CREDIT", "DEBIT") <> "12.34" Or AmountForOperation(12.34, "debit", "CREDIT") <> "-12.34" Then Err.Raise 5, , "Simultaneous type/amount paste lost canonical sign"
    If AmountForOperation(12.34, "", "DEBIT") <> "-12.34" Or AmountForOperation(12.34, "CONVERSION", "DEBIT") <> "12.34" Then Err.Raise 5, , "Draft/default or conversion sign changed"
    SelfTestOperations = "OK"
End Function

' Offline native sanity check; never touches the network or real credentials.
Public Function SelfTest() As String
    Dim parsed As Object, id As String, original As Object, missing As Object, anotherMissing As Object
    Set parsed = JsonParse("{""amount"":""123.45"",""id"":""9223372036854775807"",""a"":[null,true,""x""],""s"":""a\\b\n\u0410""}")
    If parsed("id") <> "9223372036854775807" Then Err.Raise 5, , "ID precision test failed"
    Set original = JsonClone(parsed)
    If JsonStringify(parsed) <> JsonStringify(original) Then Err.Raise 5, , "JSON round trip failed"
    If JsonStringify(1.5) <> "1.5" Or DecimalText(1.5) <> "1.5" Then Err.Raise 5, , "Decimal locale conversion failed"
    If DateText(CDbl(DateSerial(2026, 8, 31) + TimeSerial(23, 59, 58))) <> "2026-08-31T23:59:58" Then Err.Raise 5, , "Local date conversion failed"
    If DateText(CDbl(DateSerial(2026, 8, 31)), True) <> "2026-08-01" Then Err.Raise 5, , "Month conversion failed"
    Set parsed = JsonParse("{ ""a"" : 1, " & vbCrLf & " ""b"" : [ true, null ] }")
    If parsed("a") <> 1 Or parsed("b").Count <> 2 Then Err.Raise 5, , "JSON whitespace conversion failed"
    id = NewUuid()
    If Len(id) <> 36 Then Err.Raise 5, , "UUID failed"
    If Not ValidServerUrl("http://127.0.0.1:8000") Then Err.Raise 5, , "Local URL rejected"
    If ValidServerUrl("https://host/?key=bad") Then Err.Raise 5, , "Unsafe URL accepted"
    If SameGridValue(Empty, 0) Or SameGridValue(False, 0) Or SameGridValue("0", 0) Then Err.Raise 5, , "Grid comparison coerced a blank, text or Boolean"
    If Not SameGridValue(Empty, "") Or Not SameGridValue(Null, Empty) Then Err.Raise 5, , "Grid blank comparison failed"
    If SameGridValue("0012", "12") Or SameGridValue(0, 0.0000000000000001) Then Err.Raise 5, , "Grid precision comparison failed"
    If Not SameGridValue(123.45, 123.45) Or Not SameGridValue(12#, 12&) Then Err.Raise 5, , "Grid numeric comparison failed"
    If SameSnapshotValue(Null, Empty) Or SameSnapshotValue(False, 0) Or SameSnapshotValue("0", 0) Then Err.Raise 5, , "Snapshot comparison coerced JSON types"
    If SameSnapshotValue(0, 0.0000000000000001) Or SameSnapshotValue("0012", "12") Then Err.Raise 5, , "Snapshot comparison lost exact values"
    If Not SameSnapshotValue(Null, Null) Or Not SameSnapshotValue(12#, 12&) Then Err.Raise 5, , "Snapshot scalar comparison failed"
    If Not SameSnapshotValue(missing, anotherMissing) Or SameSnapshotValue(missing, Null) Then Err.Raise 5, , "Snapshot Nothing comparison failed"
    Set parsed = JsonParse("{""a"":null,""list"":[""first"",""second""],""metadata"":{""source_display"":""before""}}")
    Set original = JsonParse("{""metadata"":{""source_display"":""before""},""list"":[""first"",""second""],""a"":null}")
    If Not SameSnapshotValue(parsed, original) Then Err.Raise 5, , "Snapshot dictionary order changed equality"
    Set original("list") = JsonParse("[""second"",""first""]")
    If SameSnapshotValue(parsed, original) Then Err.Raise 5, , "Snapshot collection order was ignored"
    Set original = JsonClone(parsed): original("metadata")("source_display") = "after"
    If SameSnapshotValue(parsed, original) Then Err.Raise 5, , "Snapshot metadata-only change was ignored"
    Set original = JsonClone(parsed): original.Add "future_property", Null
    If SameSnapshotValue(parsed, original) Then Err.Raise 5, , "Snapshot extra property was ignored"
    If SelfTestOperations() <> "OK" Then Err.Raise 5, , "Operation display self-test failed"
    SelfTest = "OK: JSON, dates/months, string IDs, UUID and URL policy"
End Function

Public Function SelfTestState() As String
    ' Isolated state-machine test: no HTTP and no real workbook/client directory.
    Dim saved As Object, oldDir As String, oldBook As String, oldSelection As String, savedAliases As Object
    Dim record As Object, intent As Object, key As String, opId As Variant, firstId As String
    Dim op As Object, result As Object, serverRecord As Object, failure As String, testDir As String
    Dim snapshot As Object, snapshotRecords As Collection, newKey As String, fields As Object, original As String
    If mBusy Then Err.Raise 5, , "Cannot test during synchronization"
    Set saved = mState: oldDir = mDirectory: oldBook = mWorkbookId: oldSelection = mSelectedKey
    Set savedAliases = mRowAliases: Set mRowAliases = Nothing
    On Error GoTo Failed
    testDir = Environ$("TEMP") & "\tbs-sync-test-" & NewUuid(): MkDir testDir
    mDirectory = testDir: mWorkbookId = NewUuid()
    Set mState = Dict(): mState.Add "records", Dict(): mState.Add "intents", Dict()
    mState.Add "outbox", Dict(): mState.Add "blocked", Dict()
    Set record = Dict(): record.Add "id", "9223372036854775807": record.Add "uuid", NewUuid()
    record.Add "version", "v1": record.Add "fields", EmptyFields(): record.Add "metadata", Dict()
    record("fields")("annotation_details") = "original"
    record("fields").Remove "excel_bank": record("fields").Remove "p2p_confirmed"
    key = KeyOf(record): Records.Add key, record
    original = JsonStringify(record)
    Set fields = ViewFields(key)
    If Not IsNull(fields("excel_bank")) Or Not IsNull(fields("p2p_confirmed")) Then Err.Raise 5, , "Legacy field defaults were not null"
    If JsonStringify(record) <> original Then Err.Raise 5, , "Reading a legacy view mutated its base"
    Set intent = Dict(): intent.Add "kind", "update": intent.Add "fields", Dict()
    intent("fields").Add "annotation_details", "first"
    Intents.Add key, intent
    BuildOperations
    For Each opId In Outbox.Keys
        firstId = CStr(opId)
        Exit For
    Next
    Set op = JsonClone(Outbox.Item(firstId))
    BuildOperations
    If Outbox.Count <> 1 Then Err.Raise 5, , "Retry created another operation"
    If JsonStringify(Outbox.Item(firstId)) <> JsonStringify(op) Then Err.Raise 5, , "Retry mutated its payload"
    intent("fields")("annotation_details") = "edited while request was pending"
    Set serverRecord = JsonClone(record): serverRecord("fields")("annotation_details") = "first"
    serverRecord("version") = "v2"
    Set result = Dict(): result.Add "operation_id", firstId: result.Add "status", "applied": result.Add "record", serverRecord
    AcceptResult result
    If Intents.Item(key)("fields")("annotation_details") <> "edited while request was pending" Then Err.Raise 5, , "Acknowledgement lost a later edit"
    BuildOperations
    For Each opId In Outbox.Keys
        If CStr(opId) = firstId Then Err.Raise 5, , "New edit reused acknowledged operation ID"
        If Outbox.Item(opId)("base")("annotation_details") <> "first" Then Err.Raise 5, , "New edit did not use acknowledged base"
    Next
    Dim persisted As Object
    Set persisted = JsonParse(ReadUtf8(testDir & "\state.json"))
    If JsonStringify(persisted("outbox")) <> JsonStringify(Outbox) Then Err.Raise 5, , "Outbox was not persisted"
    original = JsonStringify(Outbox): Set fields = ViewFields(key)
    If JsonStringify(Outbox) <> original Then Err.Raise 5, , "Reading a legacy pending view mutated its retry"
    Set mState("intents") = Dict(): Set mState("outbox") = Dict()
    Set intent = Dict(): intent.Add "kind", "update": intent.Add "fields", Dict()
    intent("fields").Add "excel_bank", "HUMO Card": intent("fields").Add "p2p_confirmed", True
    Intents.Add key, intent: BuildOperations
    For Each opId In Outbox.Keys
        Set op = Outbox.Item(opId): Exit For
    Next
    If Not IsNull(op("base")("excel_bank")) Or Not IsNull(op("base")("p2p_confirmed")) Then Err.Raise 5, , "New fields did not use null legacy base"
    If VarType(op("fields")("p2p_confirmed")) <> vbBoolean Or Not CBool(op("fields")("p2p_confirmed")) Then Err.Raise 5, , "P2P confirmation was not a JSON boolean"
    Set fields = ViewFields(key)
    If fields("excel_bank") <> "HUMO Card" Or Not CBool(fields("p2p_confirmed")) Then Err.Raise 5, , "Effective bank fields lost local intent"
    If Records.Item(key)("fields").Exists("excel_bank") Or Records.Item(key)("fields").Exists("p2p_confirmed") Then Err.Raise 5, , "New-field update changed the legacy base"
    ' A snapshot after a lost create reply must not reveal an editable duplicate.
    Set mState("records") = Dict(): Set mState("intents") = Dict()
    Set mState("outbox") = Dict(): Set mState("blocked") = Dict()
    Set record = Dict(): record.Add "id", "": record.Add "uuid", NewUuid()
    record.Add "version", "": record.Add "fields", EmptyFields(): record.Add "metadata", Dict()
    key = KeyOf(record): Records.Add key, record
    Set intent = Dict(): intent.Add "kind", "create": intent.Add "fields", JsonClone(record("fields"))
    Intents.Add key, intent: BuildOperations
    For Each opId In Outbox.Keys
        firstId = CStr(opId)
        Exit For
    Next
    Set op = JsonClone(Outbox.Item(firstId))
    Set serverRecord = JsonClone(record)
    serverRecord("id") = "123": serverRecord("uuid") = NewUuid(): serverRecord("version") = "created"
    newKey = KeyOf(serverRecord)
    Set snapshot = Dict(): Set snapshotRecords = New Collection
    snapshotRecords.Add serverRecord: snapshot.Add "records", snapshotRecords: snapshot.Add "complete", True
    MergeSnapshot snapshot
    If Records.Count <> 1 Or Records.Exists(newKey) Then Err.Raise 5, , "Lost create acknowledgement exposed a duplicate"
    Set result = Dict(): result.Add "operation_id", firstId: result.Add "status", "forbidden"
    result.Add "replayed", True: result.Add "applied", True
    AcceptResult result: MergeSnapshot snapshot
    If Records.Count <> 1 Or Records.Exists(newKey) Then Err.Raise 5, , "Masked creation exposed a duplicate"
    If JsonStringify(Outbox.Item(firstId)) <> JsonStringify(op) Then Err.Raise 5, , "Masked replay mutated the create operation"
    Blocked.Remove key
    Set result = Dict(): result.Add "operation_id", firstId: result.Add "status", "applied": result.Add "record", serverRecord
    AcceptResult result: MergeSnapshot snapshot
    If Records.Count <> 1 Or Not Records.Exists(newKey) Or Outbox.Count <> 0 Then Err.Raise 5, , "Create acknowledgement did not reconcile the draft"
    Persist
    Set persisted = JsonParse(ReadUtf8(testDir & "\state.json"))
    If persisted("view_aliases")(key) <> newKey Then Err.Raise 5, , "Create view alias was not persisted"
    Set mState = persisted: Set mRowAliases = Nothing
    If ViewDisplayKey(key) <> newKey Then Err.Raise 5, , "Reopened state lost deferred bank draft anchor"
    If MergeSnapshot(snapshot) Then Err.Raise 5, , "Unchanged snapshot was treated as new state"
    ' Source labels are read-only metadata, outside the editable-field version.
    serverRecord("metadata")("source_display") = "renamed source"
    If Not MergeSnapshot(snapshot) Then Err.Raise 5, , "Metadata-only update was not merged"
    If Records.Item(newKey)("metadata")("source_display") <> "renamed source" Then Err.Raise 5, , "Metadata-only update was lost"
    If Records.Item(newKey)("version") <> "created" Then Err.Raise 5, , "Metadata update altered editable version"
    serverRecord("metadata")("source_display") = "later source"
    If Records.Item(newKey)("metadata")("source_display") <> "renamed source" Then Err.Raise 5, , "Snapshot aliases mutable local state"
    MergeSnapshot snapshot
    If MergeSnapshot(snapshot) Then Err.Raise 5, , "Merged metadata remained dirty"
    ' A complete snapshot cannot delete or replace the base of a durable edit.
    Set intent = Dict(): intent.Add "kind", "update": intent.Add "fields", Dict()
    intent("fields").Add "annotation_details", "pending local edit"
    Intents.Add newKey, intent: BuildOperations
    For Each opId In Outbox.Keys
        firstId = CStr(opId): Exit For
    Next
    Set op = JsonClone(Outbox.Item(firstId))
    Set snapshot("records") = New Collection
    If MergeSnapshot(snapshot) Or Records.Count <> 1 Then Err.Raise 5, , "Complete snapshot deleted a pending base"
    serverRecord("fields")("annotation_details") = "concurrent remote edit"
    snapshot("records").Add serverRecord
    If MergeSnapshot(snapshot) Then Err.Raise 5, , "Snapshot replaced a pending base"
    If JsonStringify(Outbox.Item(firstId)) <> JsonStringify(op) Then Err.Raise 5, , "Snapshot changed the immutable retry payload"
    Intents.Remove newKey: Outbox.Remove firstId
    Set snapshot("records") = New Collection: snapshot("complete") = False
    If MergeSnapshot(snapshot) Or Records.Count <> 1 Then Err.Raise 5, , "Incomplete snapshot deleted a record"
    snapshot("complete") = True
    If Not MergeSnapshot(snapshot) Or Records.Count <> 0 Then Err.Raise 5, , "Complete snapshot deletion was ignored"
    If MergeSnapshot(snapshot) Then Err.Raise 5, , "Repeated empty snapshot remained dirty"
    SelfTestState = "OK: immutable retries, acknowledgement rebase, later edits, durable outbox, lost-create-reply reconciliation"
    GoTo Cleanup
Failed:
    failure = Err.Description
Cleanup:
    Set mState = saved: mDirectory = oldDir: mWorkbookId = oldBook: mSelectedKey = oldSelection
    Set mRowAliases = savedAliases
    On Error Resume Next
    If Len(testDir) > 0 Then
        Kill testDir & "\state.json"
        Kill testDir & "\state.json.tmp"
        RmDir testDir
    End If
    On Error GoTo 0
    If Len(failure) > 0 Then Err.Raise 5, , failure
End Function
