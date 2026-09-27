using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;

using Lanes.Ui.Api;

namespace Lanes.Ui;

/// <summary>
/// "Spotify.exe is playing and is in no channel", with a button per channel.
/// </summary>
/// <remarks>
/// <para>
/// The new-app notification: actionable from the notice itself, and easy to
/// turn off. The core opens it (<c>--arrival &lt;executable&gt;</c>) when a
/// session appears for an application no rule places, once per application per
/// run, while notices are on in Settings and Windows says the user is not busy
/// (a full-screen game, a presentation, Focus).
/// </para>
/// <para>
/// <b>Not a Windows toast.</b> A toast with buttons needs, for an unpackaged
/// app, either a COM activator or a URL scheme registered in the registry - and
/// Lanes' one registry write is its start-on-boot entry, by design. So it is a
/// small window of its own, in the corner a toast would use.
/// </para>
/// <para>
/// <b>It never takes focus.</b> Someone in a game when a new app starts
/// playing must not be pulled out of it. It is shown without activation, stays
/// on top, and closes itself after a while - but not while the pointer is over
/// it, since that is somebody reading it.
/// </para>
/// </remarks>
public sealed class ArrivalWindow : Window
{
    /// <summary>How long it stays if nobody touches it.</summary>
    private static readonly TimeSpan Lifetime = TimeSpan.FromSeconds(20);

    private readonly CoreClient _client;
    private readonly string _executable;
    private readonly WrapPanel _choices = new() { Margin = new Thickness(0, 10, 0, 0) };
    private readonly DispatcherTimer _timer = new() { Interval = Lifetime };
    private bool _built;

    /// <summary>The app's own icon beside its name, once the state says where
    /// its executable is. See <see cref="Controls.AppIcons"/>.</summary>
    private readonly Image _icon = new()
    {
        Width = 18,
        Height = 18,
        Margin = new Thickness(0, 0, 8, 0),
        VerticalAlignment = VerticalAlignment.Center,
        Visibility = Visibility.Collapsed,
    };
    private bool _closing;

    public ArrivalWindow(string executable)
    {
        _executable = executable;

        Title = Api.CoreProcess.Titled("Lanes: new app");
        WindowStyle = WindowStyle.None;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        ShowActivated = false;
        Topmost = true;
        SizeToContent = SizeToContent.Height;
        Width = 340;
        Background = Res("Ground");
        FontFamily = (FontFamily)Application.Current.Resources["UiFont"];
        UseLayoutRounding = true;
        TextOptions.SetTextFormattingMode(this, TextFormattingMode.Ideal);
        WindowStartupLocation = WindowStartupLocation.Manual;
        Left = -10000;
        Top = -10000;

        Content = BuildContent();

        _client = new CoreClient(Dispatcher);
        _client.StateReceived += state =>
        {
            if (_built)
            {
                return;
            }
            _built = true;

            var session = state.Sessions.FirstOrDefault(
                s => s.Executable.Equals(_executable, StringComparison.OrdinalIgnoreCase));
            if (Controls.AppIcons.For(session?.Path) is { } icon)
            {
                _icon.Source = icon;
                _icon.Visibility = Visibility.Visible;
            }

            foreach (var channel in state.Channels.Where(c => !c.IsInput && c.Id != "master"))
            {
                var id = channel.Id;
                _choices.Children.Add(Choice(channel.Name, () =>
                {
                    _client.AssignApp(_executable, id);
                    CloseOnce();
                }));
            }
        };
        _client.ShuttingDown += CloseOnce;

        Loaded += async (_, _) =>
        {
            Corner.Place(this, nearPointer: false);
            _timer.Start();
            try
            {
                await _client.ConnectAsync();
            }
            catch (Exception)
            {
                // Nothing to offer without the core; go quietly.
                CloseOnce();
            }
        };
        SizeChanged += (_, _) => Dispatcher.BeginInvoke(
            () => Corner.Place(this, nearPointer: false), DispatcherPriority.Background);

        _timer.Tick += (_, _) => CloseOnce();
        MouseEnter += (_, _) => _timer.Stop();
        MouseLeave += (_, _) => _timer.Start();

        // A beat for a command sent on the way out to reach the socket.
        Closed += async (_, _) =>
        {
            await Task.Delay(120);
            await _client.DisposeAsync();
        };
    }

    private Border BuildContent()
    {
        var close = new TextBlock
        {
            Text = "×",
            FontSize = 18,
            Foreground = Res("TextFaint"),
            Cursor = Cursors.Hand,
            ToolTip = "Dismiss - it stays in To be routed",
            Margin = new Thickness(8, -4, 0, 0),
        };
        close.MouseLeftButtonUp += (_, _) => CloseOnce();
        DockPanel.SetDock(close, Dock.Right);

        var title = new TextBlock
        {
            Text = $"{_executable} is playing",
            FontWeight = FontWeights.SemiBold,
            Foreground = Res("Text"),
            TextTrimming = TextTrimming.CharacterEllipsis,
        };

        var header = new DockPanel { LastChildFill = true };
        header.Children.Add(close);
        RenderOptions.SetBitmapScalingMode(_icon, BitmapScalingMode.HighQuality);
        header.Children.Add(_icon);
        header.Children.Add(title);

        var body = new TextBlock
        {
            Text = "It is in no channel, so Lanes is not controlling it. Put it in one:",
            TextWrapping = TextWrapping.Wrap,
            FontSize = (double)Application.Current.Resources["SizeSmall"],
            Foreground = Res("TextFaint"),
            Margin = new Thickness(0, 4, 0, 0),
        };

        var ignore = new TextBlock
        {
            Text = "Ignore this app",
            FontSize = (double)Application.Current.Resources["SizeSmall"],
            Foreground = Res("TextFaint"),
            Cursor = Cursors.Hand,
            Margin = new Thickness(0, 10, 0, 0),
            ToolTip = "Never manage or show it. Undo in Settings > Ignored apps.",
        };
        ignore.MouseLeftButtonUp += (_, _) =>
        {
            _client.IgnoreApp(_executable);
            CloseOnce();
        };

        // Turning notices off from the notice itself, rather than only from
        // Settings: the moment one is unwelcome is the moment to be able to
        // say so. It is the same setting as Settings > General.
        var turnOff = new TextBlock
        {
            Text = "Turn off these notices",
            FontSize = (double)Application.Current.Resources["SizeSmall"],
            Foreground = Res("TextFaint"),
            Cursor = Cursors.Hand,
            Margin = new Thickness(16, 10, 0, 0),
            ToolTip = "Turn them back on in Settings > General.",
        };
        turnOff.MouseLeftButtonUp += (_, _) =>
        {
            _client.SetNotifyNewApps(false);
            CloseOnce();
        };

        var links = new StackPanel { Orientation = Orientation.Horizontal };
        links.Children.Add(ignore);
        links.Children.Add(turnOff);

        var layout = new StackPanel();
        layout.Children.Add(header);
        layout.Children.Add(body);
        layout.Children.Add(_choices);
        layout.Children.Add(links);

        return new Border
        {
            BorderBrush = Res("Warn"),
            BorderThickness = new Thickness(1),
            Padding = new Thickness(14, 12, 14, 12),
            Child = layout,
        };
    }

    private static Border Choice(string text, Action act)
    {
        var button = new Border
        {
            Padding = new Thickness(11, 5, 11, 5),
            Margin = new Thickness(0, 0, 6, 6),
            CornerRadius = new CornerRadius(8),
            Background = Res("Raised"),
            Cursor = Cursors.Hand,
            Child = new TextBlock { Text = text, Foreground = Res("TextDim") },
        };
        button.MouseEnter += (_, _) => button.Background = Res("LineStrong");
        button.MouseLeave += (_, _) => button.Background = Res("Raised");
        button.MouseLeftButtonUp += (_, _) => act();
        return button;
    }

    /// <summary>
    /// The one way this window closes itself. See <c>FlyoutWindow.CloseOnce</c>
    /// for why closing needs a guard at all.
    /// </summary>
    private void CloseOnce()
    {
        if (!_closing)
        {
            _closing = true;
            _timer.Stop();
            Close();
        }
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
