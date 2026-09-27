namespace Lanes.Ui;

/// <summary>
/// The window's entry point.
/// </summary>
/// <remarks>
/// This process does nothing except render state and send commands. It holds no
/// authoritative state, and it cannot reach Windows audio even by accident —
/// the core is a separate process and everything goes over 127.0.0.1.
///
/// Closing the window exits this process completely: the window is destroyed
/// rather than hidden, so its memory returns. The core keeps running in the
/// tray.
/// </remarks>
public partial class App : System.Windows.Application
{
    /// <summary>
    /// Let the last commands leave before the process does. See
    /// <see cref="Api.CoreClient.FlushAll"/>.
    /// </summary>
    protected override void OnExit(System.Windows.ExitEventArgs e)
    {
        Api.CoreClient.FlushAll(TimeSpan.FromMilliseconds(600));
        base.OnExit(e);
    }

    /// <summary>
    /// Claim the taskbar identity, apply the theme, and open the window asked for.
    /// </summary>
    protected override void OnStartup(System.Windows.StartupEventArgs e)
    {
        // The taskbar identity first. Windows reads the process's
        // AppUserModelID when the first window is created and ignores it
        // afterwards, so this cannot wait for MainWindow's constructor. It is
        // what makes the taskbar treat this window and the core as one
        // application - see Api.CoreProcess.
        Api.CoreProcess.Adopt();
        base.OnStartup(e);

        // Before any window: every colour is looked up when a control is made.
        Themes.Initialise(Api.CoreClient.SavedTheme());

        // One executable, four windows. The core starts this with --flyout when
        // the tray icon is clicked, --settings from the tray's Settings menu,
        // and --arrival <app> for a new-app notice; anything else - the Start
        // menu, the pin, the tray's "Open" - is the mixer. The first window
        // shown becomes the main window, and closing it ends the process.
        var arrival = Array.IndexOf(e.Args, "--arrival");

        System.Windows.Window window =
            e.Args.Contains("--flyout") ? new FlyoutWindow()
            : e.Args.Contains("--settings") ? new SettingsWindow()
            : arrival >= 0 && arrival + 1 < e.Args.Length ? new ArrivalWindow(e.Args[arrival + 1])
            : new MainWindow();
        window.Show();
    }
}
