Attribute VB_Name = "SyncWindows"
Option Explicit

#If VBA7 Then
Private Declare PtrSafe Function MessageBoxW Lib "user32" (ByVal owner As LongPtr, ByVal prompt As LongPtr, ByVal title As LongPtr, ByVal flags As Long) As Long
Private Declare PtrSafe Function CredWriteW Lib "advapi32" (ByRef cred As Any, ByVal flags As Long) As Long
Private Declare PtrSafe Function CredReadW Lib "advapi32" (ByVal target As LongPtr, ByVal kind As Long, ByVal flags As Long, ByRef result As LongPtr) As Long
Private Declare PtrSafe Function CredDeleteW Lib "advapi32" (ByVal target As LongPtr, ByVal kind As Long, ByVal flags As Long) As Long
Private Declare PtrSafe Sub CredFree Lib "advapi32" (ByVal memory As LongPtr)
Private Declare PtrSafe Sub CopyMemory Lib "kernel32" Alias "RtlMoveMemory" (ByRef target As Any, ByRef source As Any, ByVal count As LongPtr)
Private Declare PtrSafe Function MoveFileExW Lib "kernel32" (ByVal source As LongPtr, ByVal dest As LongPtr, ByVal flags As Long) As Long
Private Declare PtrSafe Function CoCreateGuid Lib "ole32" (ByRef value As Any) As Long
Private Declare PtrSafe Function StringFromGUID2 Lib "ole32" (ByRef value As Any, ByVal output As LongPtr, ByVal capacity As Long) As Long
Private Declare PtrSafe Function CredUIPromptForCredentialsW Lib "credui" (ByRef info As Any, ByVal target As LongPtr, ByVal reserved As LongPtr, ByVal authError As Long, ByVal user As LongPtr, ByVal userSize As Long, ByVal secret As LongPtr, ByVal secretSize As Long, ByRef save As Long, ByVal flags As Long) As Long
#Else
Private Declare Function MessageBoxW Lib "user32" (ByVal owner As Long, ByVal prompt As Long, ByVal title As Long, ByVal flags As Long) As Long
Private Declare Function CredWriteW Lib "advapi32" (ByRef cred As Any, ByVal flags As Long) As Long
Private Declare Function CredReadW Lib "advapi32" (ByVal target As Long, ByVal kind As Long, ByVal flags As Long, ByRef result As Long) As Long
Private Declare Function CredDeleteW Lib "advapi32" (ByVal target As Long, ByVal kind As Long, ByVal flags As Long) As Long
Private Declare Sub CredFree Lib "advapi32" (ByVal memory As Long)
Private Declare Sub CopyMemory Lib "kernel32" Alias "RtlMoveMemory" (ByRef target As Any, ByRef source As Any, ByVal count As Long)
Private Declare Function MoveFileExW Lib "kernel32" (ByVal source As Long, ByVal dest As Long, ByVal flags As Long) As Long
Private Declare Function CoCreateGuid Lib "ole32" (ByRef value As Any) As Long
Private Declare Function StringFromGUID2 Lib "ole32" (ByRef value As Any, ByVal output As Long, ByVal capacity As Long) As Long
Private Declare Function CredUIPromptForCredentialsW Lib "credui" (ByRef info As Any, ByVal target As Long, ByVal reserved As Long, ByVal authError As Long, ByVal user As Long, ByVal userSize As Long, ByVal secret As Long, ByVal secretSize As Long, ByRef save As Long, ByVal flags As Long) As Long
#End If

Public Function UnicodeMsgBox(ByVal prompt As String, Optional ByVal buttons As VbMsgBoxStyle = vbOKOnly, Optional ByVal title As String = "Microsoft Excel") As VbMsgBoxResult
    Dim result As Long
    ' VBA MsgBox uses the ANSI code page. Keep worksheet/server text as UTF-16.
    ' An API failure or unexpected return must never approve a pending mutation.
    UnicodeMsgBox = vbCancel
    On Error GoTo Failed
    result = MessageBoxW(Application.hWnd, StrPtr(prompt), StrPtr(title), CLng(buttons))
    If result = 0 Then Exit Function
    Select Case result
        Case vbOK, vbCancel, vbAbort, vbRetry, vbIgnore, vbYes, vbNo
            UnicodeMsgBox = result
    End Select
    Exit Function
Failed:
    UnicodeMsgBox = vbCancel
End Function

Public Function NewUuid() As String
    Dim bytes(0 To 15) As Byte, result As String
    result = String$(39, vbNullChar)
    If CoCreateGuid(bytes(0)) <> 0 Then Err.Raise 5, , "UUID generation failed"
    If StringFromGUID2(bytes(0), StrPtr(result), 39) = 0 Then Err.Raise 5, , "UUID encoding failed"
    NewUuid = LCase$(Mid$(result, 2, 36))
End Function

Private Function CredentialTarget(ByVal workbookId As String) As String
    CredentialTarget = "TBSparcer.ExcelSync." & workbookId
End Function

Public Sub SaveCredential(ByVal workbookId As String, ByVal token As String)
    Dim raw(0 To 79) As Byte, target As String, user As String, num As Long
#If VBA7 Then
    Dim ptr As LongPtr
#Else
    Dim ptr As Long
#End If
    target = CredentialTarget(workbookId): user = "Excel grant"
    num = 1: CopyMemory raw(4), num, 4
    ptr = StrPtr(target): CopyMemory raw(8), ptr, LenB(ptr)
#If Win64 Then
    num = LenB(token): CopyMemory raw(32), num, 4
    ptr = StrPtr(token): CopyMemory raw(40), ptr, 8
    num = 2: CopyMemory raw(48), num, 4
    ptr = StrPtr(user): CopyMemory raw(72), ptr, 8
#Else
    num = LenB(token): CopyMemory raw(24), num, 4
    ptr = StrPtr(token): CopyMemory raw(28), ptr, 4
    num = 2: CopyMemory raw(32), num, 4
    ptr = StrPtr(user): CopyMemory raw(48), ptr, 4
#End If
    If CredWriteW(raw(0), 0) = 0 Then Err.Raise 5, , "Windows Credential Manager write failed"
End Sub

Public Function ReadCredential(ByVal workbookId As String) As String
    Dim target As String, size As Long, value As String
#If VBA7 Then
    Dim cred As LongPtr, blob As LongPtr
#Else
    Dim cred As Long, blob As Long
#End If
    target = CredentialTarget(workbookId)
    If CredReadW(StrPtr(target), 1, 0, cred) = 0 Then Exit Function
    On Error GoTo Failed
#If Win64 Then
    CopyMemory size, ByVal cred + 32, 4
    CopyMemory blob, ByVal cred + 40, 8
#Else
    CopyMemory size, ByVal cred + 24, 4
    CopyMemory blob, ByVal cred + 28, 4
#End If
    If size < 2 Or size > 8192 Or size Mod 2 <> 0 Then Err.Raise 5, , "Invalid credential size"
    value = String$(size \ 2, vbNullChar)
    CopyMemory ByVal StrPtr(value), ByVal blob, size
    CredFree cred
    ReadCredential = value
    Exit Function
Failed:
    CredFree cred
    Err.Raise 5, , "Windows Credential Manager read failed"
End Function

Public Sub DeleteCredential(ByVal workbookId As String)
    Dim target As String
    target = CredentialTarget(workbookId)
    CredDeleteW StrPtr(target), 1, 0
End Sub

' Masked password prompt from Windows itself; VBA's InputBox would echo the
' password. Returns 0 on OK, 1223 when the user cancels, anything else when
' the dialog is unavailable (the caller then falls back to InputBox).
Public Function PromptPassword(ByVal caption As String, ByVal message As String, ByRef password As String) As Long
    Dim info() As Byte, target As String, user As String, secret As String, save As Long, result As Long
#If VBA7 Then
    Dim ptr As LongPtr
#Else
    Dim ptr As Long
#End If
    On Error GoTo Failed
    password = vbNullString
    target = "TBSparcer"
    user = "TBSparcer" & String$(514 - Len("TBSparcer"), vbNullChar)
    secret = String$(257, vbNullChar)
#If Win64 Then
    ReDim info(0 To 39)
    result = 40: CopyMemory info(0), result, 4
    ptr = Application.hWnd: CopyMemory info(8), ptr, 8
    ptr = StrPtr(message): CopyMemory info(16), ptr, 8
    ptr = StrPtr(caption): CopyMemory info(24), ptr, 8
#Else
    ReDim info(0 To 19)
    result = 20: CopyMemory info(0), result, 4
    ptr = Application.hWnd: CopyMemory info(4), ptr, 4
    ptr = StrPtr(message): CopyMemory info(8), ptr, 4
    ptr = StrPtr(caption): CopyMemory info(12), ptr, 4
#End If
    ' GENERIC_CREDENTIALS | KEEP_USERNAME | ALWAYS_SHOW_UI | DO_NOT_PERSIST
    result = CredUIPromptForCredentialsW(info(0), StrPtr(target), 0, 0, StrPtr(user), 514, _
        StrPtr(secret), 257, save, &H40000 Or &H100000 Or &H80 Or &H2)
    If result = 0 Then
        If InStr(secret, vbNullChar) > 0 Then password = Left$(secret, InStr(secret, vbNullChar) - 1) Else password = secret
    End If
    Mid$(secret, 1) = String$(Len(secret), vbNullChar)
    PromptPassword = result
    Exit Function
Failed:
    PromptPassword = -1
End Function

' A stable identity of this Windows installation and account. The token in
' the Credential Manager is per account, so two accounts are two devices and
' never revoke each other's activation.
Public Function MachineIdentity() As String
    Dim machine As String, account As String, hexed As String, i As Long
    machine = MachineGuid()
    If Len(machine) > 0 Then
        machine = "mg:" & machine
    Else
        machine = HardwareUuid()
        If Len(machine) = 0 Then Err.Raise 5, , "Cannot identify this computer"
        machine = "hw:" & machine
    End If
    account = LCase$(WindowsAccount())
    For i = 1 To Len(account)
        hexed = hexed & Right$("000" & Hex$(AscW(Mid$(account, i, 1)) And &HFFFF&), 4)
        If Len(hexed) >= 80 Then Exit For
    Next
    MachineIdentity = LCase$(machine) & ":" & LCase$(hexed)
End Function

Public Function DeviceLabel() As String
    Dim label As String
    On Error Resume Next
    label = CreateObject("WScript.Network").ComputerName
    On Error GoTo 0
    If Len(label) = 0 Then label = Environ$("COMPUTERNAME")
    If Len(label) = 0 Then label = "Windows"
    DeviceLabel = Left$(label & " (" & WindowsAccount() & ")", 80)
End Function

Private Function WindowsAccount() As String
    On Error Resume Next
    WindowsAccount = CreateObject("WScript.Network").UserName
    On Error GoTo 0
    If Len(WindowsAccount) = 0 Then WindowsAccount = Environ$("USERNAME")
End Function

Private Function MachineGuid() As String
    Dim context As Object, services As Object, registry As Object, request As Object, reply As Object
    On Error GoTo Fallback
    ' 32-bit Office reads a redirected registry view; ask the 64-bit provider.
    Set context = CreateObject("WbemScripting.SWbemNamedValueSet")
    context.Add "__ProviderArchitecture", 64
    Set services = CreateObject("WbemScripting.SWbemLocator").ConnectServer(".", "root\default", , , , , , context)
    Set registry = services.Get("StdRegProv")
    Set request = registry.Methods_("GetStringValue").InParameters.SpawnInstance_()
    request.hDefKey = 2147483650#
    request.sSubKeyName = "SOFTWARE\Microsoft\Cryptography"
    request.sValueName = "MachineGuid"
    Set reply = registry.ExecMethod_("GetStringValue", request, , context)
    If reply.ReturnValue = 0 Then MachineGuid = CleanId(reply.sValue)
    If Len(MachineGuid) > 0 Then Exit Function
Fallback:
    On Error Resume Next
    MachineGuid = CleanId(CreateObject("WScript.Shell").RegRead("HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid"))
End Function

Private Function HardwareUuid() As String
    Dim item As Object, value As String
    On Error Resume Next
    For Each item In GetObject("winmgmts:\\.\root\cimv2").ExecQuery("SELECT UUID FROM Win32_ComputerSystemProduct")
        value = CleanId(item.UUID)
        If Len(Replace(Replace(value, "f", ""), "-", "")) > 0 And Len(Replace(Replace(value, "0", ""), "-", "")) > 0 Then
            HardwareUuid = value
            Exit Function
        End If
    Next
End Function

Private Function CleanId(ByVal value As Variant) As String
    Dim text As String, i As Long, ch As String
    If IsNull(value) Or IsEmpty(value) Then Exit Function
    text = LCase$(Trim$(CStr(value)))
    For i = 1 To Len(text)
        ch = Mid$(text, i, 1)
        If Not (ch Like "[0-9a-f]" Or ch = "-") Then Exit Function
    Next
    If Len(text) >= 8 And Len(text) <= 40 Then CleanId = text
End Function

Public Function ReadUtf8(ByVal fileName As String) As String
    Dim stream As Object
    Set stream = CreateObject("ADODB.Stream")
    stream.Type = 2: stream.Charset = "utf-8": stream.Open
    stream.LoadFromFile fileName
    ReadUtf8 = stream.ReadText
    stream.Close
End Function

Public Function DecodeUtf8(ByVal bytes As Variant) As String
    Dim stream As Object
    Set stream = CreateObject("ADODB.Stream")
    stream.Type = 1: stream.Open
    stream.Write bytes
    stream.Position = 0: stream.Type = 2: stream.Charset = "utf-8"
    DecodeUtf8 = stream.ReadText
    stream.Close
End Function

Public Sub AtomicWriteUtf8(ByVal fileName As String, ByVal text As String)
    Dim stream As Object, tempName As String
    tempName = fileName & ".tmp"
    Set stream = CreateObject("ADODB.Stream")
    stream.Type = 2: stream.Charset = "utf-8": stream.Open
    stream.WriteText text
    stream.SaveToFile tempName, 2
    stream.Close
    ' MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH. Never truncate live state.
    If MoveFileExW(StrPtr(tempName), StrPtr(fileName), 9) = 0 Then Err.Raise 5, , "Cannot persist local changes"
End Sub

Public Function StateDirectory(ByVal workbookId As String) As String
    Dim fso As Object, base As String, part As Variant
    Set fso = CreateObject("Scripting.FileSystemObject")
    base = Environ$("LOCALAPPDATA")
    If Len(base) = 0 Then Err.Raise 5, , "LOCALAPPDATA is unavailable"
    For Each part In Array("TBSparcer", "ExcelSync", workbookId)
        base = base & "\" & CStr(part)
        If Not fso.FolderExists(base) Then fso.CreateFolder base
    Next
    StateDirectory = base
End Function
