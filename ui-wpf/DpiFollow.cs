using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace Lanes.Ui;

/// <summary>
/// Keeps a window the same <i>apparent</i> size when it crosses onto a display
/// with different scaling.
/// </summary>
/// <remarks>
/// <para>
/// <b>What WPF does by itself, and what it does not.</b> WPF is per-monitor DPI
/// aware, and on a DPI change it re-renders the whole visual tree at the new
/// scale — text and layout come out correct with no help at all. What it does
/// not do is resize the window: dragged from a 100% display to a 150% one, a
/// window of 1320x800 <i>physical</i> pixels stays 1320x800, which at 144 DPI
/// is only 880x533 logical. The content grows, the frame does not, and two
/// thirds of the mixer ends up outside it.
/// </para>
/// <para>
/// <b>Two approaches that do not work, recorded so they are not tried.</b>
/// </para>
/// <list type="number">
/// <item>
/// Applying the suggested rectangle from <c>WM_DPICHANGED</c> immediately, the
/// documented contract. No effect: the message arrives <i>inside Windows' modal
/// move loop</i>, which owns the window's position and size while the button is
/// down and puts it straight back on the next mouse move.
/// </item>
/// <item>
/// Re-asserting <see cref="Window.Width"/> from WPF's <c>DpiChanged</c> event,
/// queued to the dispatcher at background priority. Also no effect, for two
/// reasons: the move loop pumps the dispatcher, so "queued" still lands inside
/// it; and <c>SizeChanged</c> fires <i>before</i> <c>DpiChanged</c> with the
/// already-rescaled logical size, so the remembered size had been overwritten
/// with the wrong number before anything could use it.
/// </item>
/// </list>
/// <para>
/// So the resize is held until the move loop actually ends.
/// <c>WM_ENTERSIZEMOVE</c> and <c>WM_EXITSIZEMOVE</c> bracket it exactly, and
/// the suggested rectangle Windows provides is stored until then. A DPI change
/// that arrives outside a drag — moved by code, or the user changing the
/// display scale — is applied at once, because there is nothing to wait for.
/// </para>
/// </remarks>
internal sealed class DpiFollow
{
    private const int WmEnterSizeMove = 0x0231;
    private const int WmExitSizeMove = 0x0232;
    private const int WmDpiChanged = 0x02E0;

    private const uint SwpNoZOrder = 0x0004;
    private const uint SwpNoActivate = 0x0010;

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetWindowPos(
        IntPtr hWnd, IntPtr insertAfter, int x, int y, int cx, int cy, uint flags);

    private bool _inMoveLoop;
    private Rect? _pending;

    private DpiFollow(Window window)
    {
        if (PresentationSource.FromVisual(window) is HwndSource source)
        {
            source.AddHook(Hook);
        }
    }

    public static void Attach(Window window) => _ = new DpiFollow(window);

    private IntPtr Hook(IntPtr hwnd, int message, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        switch (message)
        {
            case WmEnterSizeMove:
                _inMoveLoop = true;
                break;

            case WmExitSizeMove:
                _inMoveLoop = false;
                if (_pending is { } waiting)
                {
                    _pending = null;
                    Apply(hwnd, waiting);
                }
                break;

            case WmDpiChanged when lParam != IntPtr.Zero:
                var suggested = Marshal.PtrToStructure<Rect>(lParam);
                if (_inMoveLoop)
                {
                    // Held until the drag ends. Applying it now would be undone
                    // by the very next mouse move.
                    _pending = suggested;
                }
                else
                {
                    Apply(hwnd, suggested);
                }
                // Never marked handled: WPF's own handler is what rescales the
                // content, and it still has to run.
                break;
        }

        return IntPtr.Zero;
    }

    private static void Apply(IntPtr hwnd, Rect target) =>
        SetWindowPos(
            hwnd,
            IntPtr.Zero,
            target.Left,
            target.Top,
            target.Right - target.Left,
            target.Bottom - target.Top,
            SwpNoZOrder | SwpNoActivate);
}
