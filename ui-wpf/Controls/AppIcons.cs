using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace Lanes.Ui.Controls;

/// <summary>
/// Applications' own icons, for the chips.
/// </summary>
/// <remarks>
/// <para>
/// <b>Read from the executable the core names</b> (<c>sessions[].path</c>). That
/// is a file on disk, not audio state, so reading it here does not give the
/// window any knowledge the API did not hand it - and the core, which has no
/// UI toolkit and wants none, would only be converting pixels for a client
/// that can do it itself.
/// </para>
/// <para>
/// <b>Extracted at 32 pixels and drawn at 16.</b> The shell's "small icon" is
/// 16 pixels at the <i>system</i> DPI, which is blurred up on the 150% display
/// this window spends half its life on. Asking for 32 and letting WPF scale
/// down is sharp on both.
/// </para>
/// <para>
/// <b>Cached for the life of the process</b>, including failures, so an
/// executable with no icon is not asked again on every state push. A few
/// applications at most, a few kilobytes each.
/// </para>
/// </remarks>
internal static class AppIcons
{
    private const int Size = 32;

    private static readonly Dictionary<string, ImageSource?> Cache = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>The icon for this executable, or null if it has none worth showing.</summary>
    public static ImageSource? For(string? path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return null;
        }

        if (!Cache.TryGetValue(path, out var icon))
        {
            icon = Extract(path);
            Cache[path] = icon;
        }

        return icon;
    }

    private static ImageSource? Extract(string path)
    {
        try
        {
            // The executable's own first icon, at the size asked for.
            if (SHDefExtractIcon(path, 0, 0, out var large, out var small, (Size << 16) | 16) == 0
                && large != IntPtr.Zero)
            {
                if (small != IntPtr.Zero)
                {
                    DestroyIcon(small);
                }

                return FromHandle(large);
            }

            // No icon of its own: the shell's generic application icon, which
            // is still a better answer than a gap in the chip.
            var info = new SHFILEINFO();
            if (SHGetFileInfo(path, 0, ref info, (uint)Marshal.SizeOf<SHFILEINFO>(), ShgfiIcon | ShgfiLargeIcon) != IntPtr.Zero
                && info.hIcon != IntPtr.Zero)
            {
                return FromHandle(info.hIcon);
            }
        }
        catch (Exception)
        {
            // A broken or inaccessible executable is not worth a crash; the chip
            // shows its name either way.
        }

        return null;
    }

    private static ImageSource FromHandle(IntPtr handle)
    {
        try
        {
            var source = Imaging.CreateBitmapSourceFromHIcon(
                handle, Int32Rect.Empty, BitmapSizeOptions.FromEmptyOptions());
            source.Freeze();
            return source;
        }
        finally
        {
            DestroyIcon(handle);
        }
    }

    private const uint ShgfiIcon = 0x100;
    private const uint ShgfiLargeIcon = 0x0;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct SHFILEINFO
    {
        public IntPtr hIcon;
        public int iIcon;
        public uint dwAttributes;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)]
        public string szDisplayName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 80)]
        public string szTypeName;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern int SHDefExtractIcon(
        string iconFile, int iconIndex, uint flags, out IntPtr large, out IntPtr small, uint iconSize);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr SHGetFileInfo(
        string path, uint attributes, ref SHFILEINFO info, uint size, uint flags);

    [DllImport("user32.dll")]
    private static extern bool DestroyIcon(IntPtr icon);
}
