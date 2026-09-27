using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace Lanes.Ui;

/// <summary>
/// Reads and restores the window's size, position and monitor.
/// </summary>
/// <remarks>
/// <para>
/// <b>Win32 placement rather than WPF's Left/Top/Width/Height.</b> Those are
/// logical units, and a logical size means nothing without saying which
/// monitor's scaling it was logical in — save 1320 wide on a 150% display and
/// restore it on a 100% one and the window is two thirds the size it was.
/// </para>
/// <para>
/// <c>GetWindowPlacement</c> works in physical pixels on the virtual screen and
/// gives the <i>restore</i> rectangle even when the window is maximised, so
/// un-maximising later lands somewhere sensible. <c>SetWindowPlacement</c>
/// handles the per-monitor DPI change that restoring onto a different display
/// implies, which is precisely the code this window exists to avoid writing
/// twice.
/// </para>
/// </remarks>
internal static class Placement
{
    private const int SW_SHOWNORMAL = 1;
    private const int SW_SHOWMAXIMIZED = 3;
    private const int SW_SHOWMINIMIZED = 2;

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT
    {
        public int X;
        public int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct WINDOWPLACEMENT
    {
        public int length;
        public int flags;
        public int showCmd;
        public POINT minPosition;
        public POINT maxPosition;
        public RECT normalPosition;
    }

    [DllImport("user32.dll")]
    private static extern bool GetWindowPlacement(IntPtr window, ref WINDOWPLACEMENT placement);

    [DllImport("user32.dll")]
    private static extern bool SetWindowPlacement(IntPtr window, ref WINDOWPLACEMENT placement);

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromRect(ref RECT rect, uint flags);

    /// <summary>MONITOR_DEFAULTTONULL: no monitor overlaps this rectangle.</summary>
    private const uint MonitorDefaultToNull = 0;

    /// <summary>What the window is doing right now, for storing.</summary>
    public readonly record struct Bounds(int X, int Y, int Width, int Height, bool Maximised);

    /// <summary>Read the window's restore rectangle, or null if it has no handle yet.</summary>
    public static Bounds? Read(Window window)
    {
        var handle = new WindowInteropHelper(window).Handle;
        if (handle == IntPtr.Zero)
        {
            return null;
        }

        var placement = new WINDOWPLACEMENT();
        placement.length = Marshal.SizeOf<WINDOWPLACEMENT>();
        if (!GetWindowPlacement(handle, ref placement))
        {
            return null;
        }

        var r = placement.normalPosition;

        // A minimised window is not a size worth remembering, but its restore
        // rectangle is, and that is what normalPosition already holds.
        var maximised = placement.showCmd == SW_SHOWMAXIMIZED;

        return new Bounds(r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top, maximised);
    }

    /// <summary>
    /// Put the window back where it was, if that is still somewhere real.
    /// </summary>
    /// <returns>
    /// False when the stored position is off every current display, so the
    /// caller can fall back rather than open a window nobody can see.
    /// </returns>
    /// <remarks>
    /// If that monitor is absent next launch, the window falls back to the
    /// primary display rather than opening off-screen. A laptop undocked
    /// between sessions is the ordinary way it
    /// happens, and a window restored onto a monitor that is gone is not merely
    /// inconvenient — there is no way to reach it with a mouse.
    /// </remarks>
    public static bool Restore(Window window, Bounds bounds)
    {
        var handle = new WindowInteropHelper(window).Handle;
        if (handle == IntPtr.Zero || bounds.Width <= 0 || bounds.Height <= 0)
        {
            return false;
        }

        var rect = new RECT
        {
            Left = bounds.X,
            Top = bounds.Y,
            Right = bounds.X + bounds.Width,
            Bottom = bounds.Y + bounds.Height,
        };

        // Asked before restoring, not after. MonitorFromRect with
        // DEFAULTTONULL is the direct question - "does any display overlap this
        // rectangle?" - and answering it first means an off-screen window is
        // never shown at all, rather than shown and then moved.
        if (MonitorFromRect(ref rect, MonitorDefaultToNull) == IntPtr.Zero)
        {
            return false;
        }

        var placement = new WINDOWPLACEMENT
        {
            length = Marshal.SizeOf<WINDOWPLACEMENT>(),
            flags = 0,
            showCmd = bounds.Maximised ? SW_SHOWMAXIMIZED : SW_SHOWNORMAL,
            normalPosition = rect,
        };

        return SetWindowPlacement(handle, ref placement);
    }
}
