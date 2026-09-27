using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace Lanes.Ui;

/// <summary>
/// Put a small window in a corner of the screen, inside the work area.
/// </summary>
/// <remarks>
/// <para>
/// Shared by the quick mixer and the new-app notice, which both live beside
/// the notification area.
/// </para>
/// <para>
/// In physical pixels, from Win32, for the same reason the mixer's placement
/// is: a monitor's work area is a fact in physical pixels, and converting it
/// through WPF's logical units would mean knowing that monitor's scaling before
/// the window is on it.
/// </para>
/// <para>
/// <b>Call it after the native window has its final size.</b> WPF raises
/// <c>SizeChanged</c> before the native window has been resized, so a
/// rectangle read in that handler is the old one - which is how the quick
/// mixer's first version anchored a 90-pixel window above the taskbar and then
/// grew 800 pixels downwards off the screen. Defer to Background priority.
/// </para>
/// </remarks>
internal static class Corner
{
    private const int EdgeGap = 12;

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    private struct MONITORINFO
    {
        public int cbSize;
        public RECT rcMonitor;
        public RECT rcWork;
        public int dwFlags;
    }

    [DllImport("user32.dll")] private static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(POINT point, uint flags);
    [DllImport("user32.dll")] private static extern bool GetMonitorInfo(IntPtr monitor, ref MONITORINFO info);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")]
    private static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);

    /// <summary>Place the window.</summary>
    /// <param name="nearPointer">
    /// True: the corner of the pointer's monitor nearest the pointer - right
    /// for the quick mixer, whose pointer has just clicked the tray icon, so it
    /// lands beside the taskbar wherever the taskbar is. False: the bottom
    /// right of the primary monitor, where the notification area normally is -
    /// right for a notice nobody clicked to summon.
    /// </param>
    public static void Place(Window window, bool nearPointer)
    {
        var hwnd = new WindowInteropHelper(window).Handle;
        if (hwnd == IntPtr.Zero || !GetWindowRect(hwnd, out var own))
        {
            return;
        }

        var anchor = new POINT();
        if (!nearPointer || !GetCursorPos(out anchor))
        {
            anchor = new POINT(); // (0,0) is on the primary monitor, always.
        }

        var info = new MONITORINFO { cbSize = Marshal.SizeOf<MONITORINFO>() };
        // MONITOR_DEFAULTTONEAREST / MONITOR_DEFAULTTOPRIMARY
        if (!GetMonitorInfo(MonitorFromPoint(anchor, nearPointer ? 2u : 1u), ref info))
        {
            return;
        }

        var work = info.rcWork;
        var width = own.Right - own.Left;
        var height = own.Bottom - own.Top;

        var bottom = !nearPointer || anchor.Y > (work.Top + work.Bottom) / 2;
        var right = !nearPointer || anchor.X > (work.Left + work.Right) / 2;

        var x = right ? work.Right - width - EdgeGap : work.Left + EdgeGap;
        var y = bottom ? work.Bottom - height - EdgeGap : work.Top + EdgeGap;

        // SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
        SetWindowPos(hwnd, IntPtr.Zero, x, y, 0, 0, 0x0001 | 0x0004 | 0x0010);
    }
}
