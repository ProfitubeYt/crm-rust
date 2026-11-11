Attribute VB_Name = "SyncJson"
Option Explicit

' Self-contained JSON. IDs and decimal amounts travel as JSON strings.
Private pText As String
Private pPos As Long

Public Function JsonParse(ByVal text As String) As Object
    pText = text: pPos = 1
    SkipSpace
    If Mid$(pText, pPos, 1) = "{" Then
        Set JsonParse = ReadObject()
    ElseIf Mid$(pText, pPos, 1) = "[" Then
        Set JsonParse = ReadArray()
    Else
        Err.Raise 5, , "JSON root must be an object or array"
    End If
    SkipSpace
    If pPos <= Len(pText) Then Err.Raise 5, , "Unexpected trailing JSON"
End Function

Public Function Dict() As Object
    Set Dict = CreateObject("Scripting.Dictionary")
    Dict.CompareMode = vbBinaryCompare
End Function

Public Function JsonClone(ByVal value As Object) As Object
    Set JsonClone = JsonParse(JsonStringify(value))
End Function

Private Sub SkipSpace()
    Do While pPos <= Len(pText)
        If InStr(1, " " & vbTab & vbCr & vbLf, Mid$(pText, pPos, 1), vbBinaryCompare) = 0 Then Exit Do
        pPos = pPos + 1
    Loop
End Sub

Private Sub Expect(ByVal ch As String)
    SkipSpace
    If Mid$(pText, pPos, 1) <> ch Then Err.Raise 5, , "Invalid JSON at " & pPos
    pPos = pPos + 1
End Sub

Private Function ReadObject() As Object
    Dim result As Object, key As String, item As Variant
    Set result = Dict(): Expect "{": SkipSpace
    If Mid$(pText, pPos, 1) = "}" Then
        pPos = pPos + 1: Set ReadObject = result: Exit Function
    End If
    Do
        Call SkipSpace: key = ReadString(): Expect ":"
        If result.Exists(key) Then Err.Raise 5, , "Duplicate JSON property: " & key
        ReadInto result, key
        SkipSpace
        If Mid$(pText, pPos, 1) = "}" Then Exit Do
        Expect ","
    Loop
    pPos = pPos + 1: Set ReadObject = result
End Function

Private Sub ReadInto(ByVal target As Object, ByVal key As String)
    Dim value As Variant
    SkipSpace
    Select Case Mid$(pText, pPos, 1)
        Case "{": target.Add key, ReadObject()
        Case "[": target.Add key, ReadArray()
        Case Else: target.Add key, ReadScalar()
    End Select
End Sub

Private Function ReadArray() As Collection
    Dim result As New Collection
    Expect "[": SkipSpace
    If Mid$(pText, pPos, 1) = "]" Then
        pPos = pPos + 1: Set ReadArray = result: Exit Function
    End If
    Do
        SkipSpace
        Select Case Mid$(pText, pPos, 1)
            Case "{": result.Add ReadObject()
            Case "[": result.Add ReadArray()
            Case Else: result.Add ReadScalar()
        End Select
        SkipSpace
        If Mid$(pText, pPos, 1) = "]" Then Exit Do
        Expect ","
    Loop
    pPos = pPos + 1: Set ReadArray = result
End Function

Private Function ReadString() As String
    Dim ch As String, answer As String, code As Long
    Expect Chr$(34)
    Do While pPos <= Len(pText)
        ch = Mid$(pText, pPos, 1): pPos = pPos + 1
        If ch = Chr$(34) Then ReadString = answer: Exit Function
        If ch = "\" Then
            ch = Mid$(pText, pPos, 1): pPos = pPos + 1
            Select Case ch
                Case Chr$(34), "\", "/": answer = answer & ch
                Case "b": answer = answer & Chr$(8)
                Case "f": answer = answer & Chr$(12)
                Case "n": answer = answer & vbLf
                Case "r": answer = answer & vbCr
                Case "t": answer = answer & vbTab
                Case "u"
                    code = CLng("&H" & Mid$(pText, pPos, 4)): pPos = pPos + 4
                    If code > 32767 Then code = code - 65536
                    answer = answer & ChrW$(code)
                Case Else: Err.Raise 5, , "Invalid JSON escape"
            End Select
        Else
            If AscW(ch) >= 0 And AscW(ch) < 32 Then Err.Raise 5, , "Invalid JSON control character"
            answer = answer & ch
        End If
    Loop
    Err.Raise 5, , "Unterminated JSON string"
End Function

Private Function ReadScalar() As Variant
    Dim start As Long, token As String, ch As String
    Call SkipSpace: ch = Mid$(pText, pPos, 1)
    If ch = Chr$(34) Then ReadScalar = ReadString(): Exit Function
    start = pPos
    Do While pPos <= Len(pText)
        ch = Mid$(pText, pPos, 1)
        If InStr(1, ",]} " & vbCr & vbLf & vbTab, ch, vbBinaryCompare) > 0 Then Exit Do
        pPos = pPos + 1
    Loop
    token = Mid$(pText, start, pPos - start)
    Select Case token
        Case "true": ReadScalar = True
        Case "false": ReadScalar = False
        Case "null": ReadScalar = Null
        Case Else
            Dim pattern As Object
            Set pattern = CreateObject("VBScript.RegExp")
            pattern.Pattern = "^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$"
            If Not pattern.Test(token) Then Err.Raise 5, , "Invalid JSON value"
            ReadScalar = Val(token)
    End Select
End Function

Public Function JsonQuote(ByVal text As String) As String
    Dim i As Long, ch As String, code As Long, answer As String
    For i = 1 To Len(text)
        ch = Mid$(text, i, 1): code = AscW(ch)
        Select Case ch
            Case Chr$(34): answer = answer & "\" & Chr$(34)
            Case "\": answer = answer & "\\"
            Case vbCr: answer = answer & "\r"
            Case vbLf: answer = answer & "\n"
            Case vbTab: answer = answer & "\t"
            Case Else
                If code >= 0 And code < 32 Then
                    answer = answer & "\u" & Right$("0000" & Hex$(code), 4)
                Else
                    answer = answer & ch
                End If
        End Select
    Next
    JsonQuote = Chr$(34) & answer & Chr$(34)
End Function

Public Function JsonStringify(ByVal value As Variant) As String
    Dim answer As String, key As Variant, item As Variant
    If IsObject(value) Then
        If TypeName(value) = "Collection" Then
            For Each item In value
                If Len(answer) > 0 Then answer = answer & ","
                answer = answer & JsonStringify(item)
            Next
            JsonStringify = "[" & answer & "]"
        Else
            For Each key In value.Keys
                If Len(answer) > 0 Then answer = answer & ","
                answer = answer & JsonQuote(CStr(key)) & ":" & JsonStringify(value(key))
            Next
            JsonStringify = "{" & answer & "}"
        End If
    ElseIf IsNull(value) Or IsEmpty(value) Then
        JsonStringify = "null"
    ElseIf VarType(value) = vbBoolean Then
        If value Then JsonStringify = "true" Else JsonStringify = "false"
    ElseIf VarType(value) = vbString Then
        JsonStringify = JsonQuote(CStr(value))
    ElseIf IsNumeric(value) Then
        ' CStr follows VBA/Windows locale, even if Excel uses custom separators.
        JsonStringify = Replace$(CStr(value), Mid$(CStr(1.5), 2, 1), ".")
    Else
        Err.Raise 5, , "Unsupported JSON type"
    End If
End Function

Public Function SameValue(ByVal a As Variant, ByVal b As Variant) As Boolean
    SameValue = (JsonStringify(a) = JsonStringify(b))
End Function

Public Function TextValue(ByVal value As Variant) As String
    If Not IsNull(value) And Not IsEmpty(value) Then TextValue = CStr(value)
End Function
