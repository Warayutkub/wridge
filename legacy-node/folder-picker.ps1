# The Windows "Select Folder" dialog (the File Explorer one, IFileOpenDialog with FOS_PICKFOLDERS).
# Prints the chosen folder, or nothing when cancelled. Run with -STA. Keep this file ASCII:
# Windows PowerShell 5.1 reads a .ps1 without a BOM in the ANSI code page.
param([string]$Title = 'Choose a folder', [string]$Start = '')

[Console]::OutputEncoding = [Text.Encoding]::UTF8
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;

public static class WaraFolderPicker {
  [ComImport, Guid("DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7")] class FileOpenDialogRCW {}

  // IFileDialog up to GetResult, in vtable order (IModalWindow::Show first).
  [ComImport, Guid("42f85136-db7e-439c-85f1-e4075d135fc8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IFileDialog {
    [PreserveSig] int Show(IntPtr parent);
    void SetFileTypes(uint count, IntPtr specs);
    void SetFileTypeIndex(uint index);
    void GetFileTypeIndex(out uint index);
    void Advise(IntPtr events, out uint cookie);
    void Unadvise(uint cookie);
    void SetOptions(uint options);
    void GetOptions(out uint options);
    void SetDefaultFolder(IShellItem item);
    void SetFolder(IShellItem item);
    void GetFolder(out IShellItem item);
    void GetCurrentSelection(out IShellItem item);
    void SetFileName([MarshalAs(UnmanagedType.LPWStr)] string name);
    void GetFileName([MarshalAs(UnmanagedType.LPWStr)] out string name);
    void SetTitle([MarshalAs(UnmanagedType.LPWStr)] string title);
    void SetOkButtonLabel([MarshalAs(UnmanagedType.LPWStr)] string text);
    void SetFileNameLabel([MarshalAs(UnmanagedType.LPWStr)] string label);
    void GetResult(out IShellItem item);
  }

  [ComImport, Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IShellItem {
    void BindToHandler(IntPtr bc, ref Guid bhid, ref Guid riid, out IntPtr ppv);
    void GetParent(out IShellItem parent);
    void GetDisplayName(uint sigdn, [MarshalAs(UnmanagedType.LPWStr)] out string name);
    void GetAttributes(uint mask, out uint attributes);
    void Compare(IShellItem other, uint hint, out int order);
  }

  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr bc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, out IShellItem item);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();

  const uint FOS_PICKFOLDERS = 0x20, FOS_FORCEFILESYSTEM = 0x40;
  const uint SIGDN_FILESYSPATH = 0x80058000;

  public static string Pick(string title, string start) {
    var dialog = (IFileDialog)new FileOpenDialogRCW();
    uint options;
    dialog.GetOptions(out options);
    dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
    dialog.SetTitle(title);
    // open at the start folder, or the nearest parent that exists
    string dir = string.IsNullOrEmpty(start) ? null : start.Replace('/', '\\');
    while (!string.IsNullOrEmpty(dir) && !Directory.Exists(dir)) dir = Path.GetDirectoryName(dir);
    if (!string.IsNullOrEmpty(dir)) {
      IShellItem folder;
      SHCreateItemFromParsingName(dir, IntPtr.Zero, typeof(IShellItem).GUID, out folder);
      dialog.SetFolder(folder);
    }
    // owned by the window in front (the browser the user just clicked in), so it opens on top of it
    if (dialog.Show(GetForegroundWindow()) != 0) return null;
    IShellItem result;
    dialog.GetResult(out result);
    string path;
    result.GetDisplayName(SIGDN_FILESYSPATH, out path);
    return path;
  }
}
'@
if ($Title -eq '--compile-only') { 'compiled'; exit 0 }
$picked = [WaraFolderPicker]::Pick($Title, $Start)
if ($picked) { $picked }
