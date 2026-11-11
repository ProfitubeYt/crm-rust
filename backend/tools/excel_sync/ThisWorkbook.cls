VERSION 1.0 CLASS
BEGIN
  MultiUse = -1
END
Attribute VB_Name = "ThisWorkbook"
Option Explicit

Private Sub Workbook_Open()
    ExcelSync.StartSync
End Sub

Private Sub Workbook_Activate()
    ExcelSync.ResumeView
End Sub

Private Sub Workbook_BeforeClose(Cancel As Boolean)
    Cancel = Not ExcelSync.PrepareClose()
End Sub

Private Sub Workbook_SheetChange(ByVal Sh As Object, ByVal Target As Range)
    If BankViews.IsBankSheet(Sh.Name) Then
        BankViews.CellsEdited Target
    ElseIf Sh.Name = ExcelSync.DataSheetName() Then
        ExcelSync.CellsEdited Target
    End If
End Sub

Private Sub Workbook_SheetActivate(ByVal Sh As Object)
    If BankViews.IsBankSheet(Sh.Name) Then ExcelSync.ResumeView
End Sub

Private Sub Workbook_BeforeSave(ByVal SaveAsUI As Boolean, Cancel As Boolean)
    ExcelSync.HandleBeforeSave SaveAsUI, Cancel
End Sub

Private Sub Workbook_AfterSave(ByVal Success As Boolean)
    ExcelSync.AfterSaved Success
End Sub

Private Sub Workbook_SheetSelectionChange(ByVal Sh As Object, ByVal Target As Range)
    If BankViews.IsBankSheet(Sh.Name) Then
        BankViews.SelectionChanged Target
    ElseIf Sh.Name = ExcelSync.DataSheetName() Then
        ExcelSync.SelectionChanged Target
    End If
End Sub

Private Sub Workbook_SheetBeforeDoubleClick(ByVal Sh As Object, ByVal Target As Range, Cancel As Boolean)
    If BankViews.IsBankSheet(Sh.Name) Then
        BankViews.DoubleClick Target, Cancel
    ElseIf Sh.Name = ExcelSync.DataSheetName() And Target.Column = 50 And Target.Row >= 5 Then
        Cancel = True
        ExcelSync.RowAction Target.Row
    ElseIf Sh.Name = ExcelSync.LockSheetName() Then
        Cancel = True
        ExcelSync.LockSheetAction
    End If
End Sub
