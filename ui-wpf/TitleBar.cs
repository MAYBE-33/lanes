using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace Lanes.Ui;

/// <summary>
/// Matches the window's own title bar to the palette in use.
/// </summary>
/// <remarks>
/// <para>
/// The title bar is drawn by the desktop window manager, not by WPF, so it
/// follows the <i>system</i> light/dark setting and ignores everything in
/// <c>Theme.xaml</c>. With the system on light and the window dark, that left a
/// white caption bar sitting on top of a near-black mixer, which is the single
/// loudest thing in a window whose whole design brief was "quiet" - and since
/// the window can be set to either regardless of Windows (see
/// <see cref="Themes"/>), the two can disagree in both directions.
/// </para>
/// <para>
/// <c>DWMWA_USE_IMMERSIVE_DARK_MODE</c> is the documented way to say
/// otherwise. It was attribute 19 on Windows 10 builds before 20H1 and 20 from
/// then on; both are tried, newest first, and a build that knows neither
/// simply returns a failure and keeps its light title bar. There is nothing to
/// recover from — the window is perfectly usable either way — so the result is
/// deliberately not treated as an error.
/// </para>
/// <para>
/// This is the app following the window's own appearance to its content, not
/// the app overriding the user's system preference: nothing outside this
/// window is affected.
/// </para>
/// </remarks>
internal static class TitleBar
{
    private const int UseImmersiveDarkMode = 20;
    private const int UseImmersiveDarkModeBefore20H1 = 19;

    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(
        IntPtr window, int attribute, ref int value, int size);

    /// <summary>Match the caption bar to the palette in use.</summary>
    public static void MakeDark(Window window) => Match(window, dark: !Themes.Light);

    private static void Match(Window window, bool dark)
    {
        var handle = new WindowInteropHelper(window).Handle;
        if (handle == IntPtr.Zero)
        {
            return;
        }

        var on = dark ? 1 : 0;
        if (DwmSetWindowAttribute(handle, UseImmersiveDarkMode, ref on, sizeof(int)) != 0)
        {
            DwmSetWindowAttribute(handle, UseImmersiveDarkModeBefore20H1, ref on, sizeof(int));
        }
    }
}
