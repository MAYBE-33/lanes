using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;
using Lanes.Ui.Controls;

namespace Lanes.Ui;

/// <summary>
/// Settings that do not belong on the mixer: channels, hotkeys, ignored apps,
/// notifications, backup and diagnostics.
/// </summary>
/// <remarks>
/// <para>
/// <b>A window of its own rather than a panel in the mixer.</b> The mixer's
/// header band has no room left - the pool, the devices, the mix, the override
/// and the profile picker already share it - and the mixer's standing rule is
/// that it shows the mix and nothing that is not about the mix. Hotkeys are set
/// once and left. So this is the same executable run with <c>--settings</c>,
/// opened from the tray, a pure client of the core like the mixer and the
/// flyout, and torn down when it closes.
/// </para>
/// <para>
/// <b>Every change is saved as it is made</b>, the moment a row is complete.
/// There is no Save button to forget. The core checks each list it is sent and
/// refuses it with a reason - "Shift+M needs Ctrl, Alt or Win with it" - which
/// is shown here, and a row the core has not accepted says so rather than
/// looking saved.
/// </para>
/// </remarks>
public sealed class SettingsWindow : Window
{
    private readonly CoreClient _client;
    private readonly StackPanel _rows = new();
    private readonly TextBlock _problem = new();
    private readonly List<Row> _model = [];

    private IReadOnlyList<ChannelState> _channels = [];
    private IReadOnlyList<string> _profiles = [];
    private IReadOnlyList<HotkeyState> _live = [];
    private bool _built;

    /// <summary>A list has been sent and the core has not answered yet.</summary>
    private bool _saving;

    private readonly ContentControl _page = new();
    private readonly StackPanel _tabs = new() { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 18) };
    private readonly StackPanel _ignoredList = new();
    private readonly Toggle _notify = new();
    private readonly TextBlock _backupMessage = new();
    private readonly StackPanel _diagnostics = new();
    private IReadOnlyList<string> _ignored = [];
    private readonly StackPanel _channelList = new();
    private readonly TextBox _newChannel = new();
    private readonly StackPanel _addChannelRow = new() { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 14, 0, 0) };
    private readonly TextBlock _channelsFull = new();
    private readonly TextBlock _channelProblem = new();
    private readonly StackPanel _themeChoice = new() { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 26) };
    private readonly TextBlock _restoreState = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 8, 0, 0) };
    private readonly Button _resume = new() { Content = "Resume Lanes", Visibility = Visibility.Collapsed };
    private bool _paused;

    /// <summary>The tab on show, so a rebuild for a theme change can reopen it.</summary>
    private string _currentTab = "Channels";

    /// <summary>Every action a hotkey can have, in the order they are offered.</summary>
    private static readonly (string Kind, string Label, Target Needs)[] Actions =
    [
        ("toggle_mute", "Mute / unmute", Target.Channel),
        ("volume_up", "Volume up 5%", Target.Channel),
        ("volume_down", "Volume down 5%", Target.Channel),
        ("cycle_device", "Next output device", Target.OutputChannel),
        ("mute_all", "Mute all", Target.None),
        ("activate_profile", "Switch profile", Target.Profile),
        ("show_window", "Show / hide the mixer", Target.None),
        ("quick_mixer", "Quick mixer", Target.None),
    ];

    private enum Target { None, Channel, OutputChannel, Profile }

    /// <summary>One row being edited. Complete rows are what the core is sent.</summary>
    private sealed class Row
    {
        public string? Keys;
        public string? Kind;
        public string? Target;
        public TextBlock Status = new();
        public ChoiceButton TargetChoice = new();
    }

    /// <param name="openTab">The tab to show first. Used when the window is
    /// rebuilt for a theme change, so the user stays where they were.</param>
    public SettingsWindow(string? openTab = null)
    {
        Title = Api.CoreProcess.Titled("Lanes settings");
        Width = 720;
        Height = 520;
        MinWidth = 560;
        MinHeight = 320;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        Background = Res("Ground");
        FontFamily = (FontFamily)Application.Current.Resources["UiFont"];
        UseLayoutRounding = true;
        TextOptions.SetTextFormattingMode(this, TextFormattingMode.Ideal);

        Content = BuildContent(openTab);

        _client = new CoreClient(Dispatcher);
        _client.StateReceived += state =>
        {
            _saving = false;
            _channels = state.Channels;
            _profiles = state.Profiles?.Names ?? [];
            _live = state.Hotkeys ?? [];
            _ignored = state.Ignored ?? [];
            _notify.IsOn = state.Settings?.NotifyNewApps ?? false;
            Themes.Choose(state.Settings?.Theme);
            RefreshTheme();
            ShowPaused(state.Settings?.Paused == true);
            RefreshIgnored();
            RefreshChannels();
            if (!_built)
            {
                _built = true;
                BuildRows();
            }
            RefreshTargets();
            RefreshStatus();
        };
        _client.DeltaReceived += delta =>
        {
            if (delta.Channels is { } channels)
            {
                _channels = channels;
                RefreshTargets();
                RefreshChannels();
            }
            if (delta.Profiles is { } profiles)
            {
                _profiles = profiles.Names;
                RefreshTargets();
            }
            if (delta.Hotkeys is { } hotkeys)
            {
                _live = hotkeys;
                RefreshStatus();
            }
            if (delta.Ignored is { } ignored)
            {
                _ignored = ignored;
                RefreshIgnored();
            }
            if (delta.Settings is { } settings)
            {
                _notify.IsOn = settings.NotifyNewApps;
                Themes.Choose(settings.Theme);
                RefreshTheme();
                ShowPaused(settings.Paused);
            }
        };
        _client.DiagnosticsReceived += ShowDiagnostics;
        _client.ConfigExported += SaveExport;
        _client.CommandRefused += why =>
        {
            _saving = false;
            _problem.Text = why;
            _backupMessage.Text = why;
            _channelProblem.Text = why;
            _backupMessage.Foreground = Res("Warn");
            RefreshStatus();
        };
        _client.ShuttingDown += Close;

        Loaded += async (_, _) =>
        {
            try
            {
                await _client.ConnectAsync();
            }
            catch (Exception ex)
            {
                _problem.Text = ex.Message;
            }
        };

        Themes.Changed += Rebuild;
        Closed += async (_, _) =>
        {
            Themes.Changed -= Rebuild;
            await _client.DisposeAsync();
        };
    }

    /// <summary>
    /// The palette changed: replace this window with one built in the new
    /// colours, in the same place and on the same tab. See <see cref="Themes"/>.
    /// </summary>
    private void Rebuild()
    {
        var replacement = new SettingsWindow(_currentTab)
        {
            WindowStartupLocation = WindowStartupLocation.Manual,
            Left = Left,
            Top = Top,
            Width = Width,
            Height = Height,
        };

        if (Application.Current.MainWindow == this)
        {
            Application.Current.MainWindow = replacement;
        }

        replacement.Show();
        Close();
    }

    protected override void OnSourceInitialized(EventArgs e)
    {
        base.OnSourceInitialized(e);
        TitleBar.MakeDark(this);
    }

    /// <summary>The tabs and the page under them.</summary>
    /// <remarks>
    /// Each page is built once and kept, so switching tabs does not throw away
    /// a hotkey half-entered on another one.
    /// </remarks>
    private UIElement BuildContent(string? openTab)
    {
        var pages = new (string Name, UIElement Page)[]
        {
            ("Channels", Scrolling(BuildChannelsPage())),
            ("Hotkeys", Scrolling(BuildHotkeysPage())),
            ("Ignored apps", Scrolling(BuildIgnoredPage())),
            ("General", Scrolling(BuildGeneralPage())),
            ("Backup", Scrolling(BuildBackupPage())),
            ("Diagnostics", Scrolling(BuildDiagnosticsPage())),
        };

        foreach (var (name, page) in pages)
        {
            var label = new TextBlock
            {
                Text = name,
                FontSize = (double)Application.Current.Resources["SizeBody"],
            };
            var tab = new Border
            {
                Padding = new Thickness(12, 6, 12, 6),
                Margin = new Thickness(0, 0, 6, 0),
                CornerRadius = new CornerRadius(8),
                Cursor = Cursors.Hand,
                Child = label,
            };
            tab.MouseLeftButtonUp += (_, _) => Show(name, page);
            _tabs.Children.Add(tab);
        }

        var first = pages.FirstOrDefault(p => p.Name == openTab);
        if (first.Page is null)
        {
            first = pages[0];
        }
        Show(first.Name, first.Page);

        var layout = new DockPanel { LastChildFill = true, Margin = new Thickness(24, 18, 24, 20) };
        DockPanel.SetDock(_tabs, Dock.Top);
        layout.Children.Add(_tabs);
        layout.Children.Add(_page);
        return layout;
    }

    /// <summary>
    /// A page that scrolls when it is taller than the window.
    /// </summary>
    /// <remarks>
    /// Every page is wrapped in one, because General, at the window's default
    /// size, is taller than the window. A page that cannot be reached is a
    /// setting that does not exist.
    /// </remarks>
    private static ScrollViewer Scrolling(UIElement page) => new()
    {
        Content = page,
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
        HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        Padding = new Thickness(0, 0, 6, 0),
    };

    private void Show(string name, UIElement page)
    {
        _currentTab = name;
        _page.Content = page;
        foreach (Border tab in _tabs.Children)
        {
            var chosen = ((TextBlock)tab.Child).Text == name;
            tab.Background = Res(chosen ? "Raised" : "Ground");
            ((TextBlock)tab.Child).Foreground = Res(chosen ? "Text" : "TextFaint");
        }

        // Diagnostics is a snapshot; opening the tab takes a fresh one.
        if (name == "Diagnostics")
        {
            _client.GetDiagnostics();
        }
    }

    private static TextBlock PageHeading(string text) => new()
    {
        Text = text,
        FontSize = (double)Application.Current.Resources["SizeChannel"],
        FontWeight = FontWeights.SemiBold,
        Foreground = Res("Text"),
    };

    private static TextBlock Explanation(string text) => new()
    {
        Text = text,
        TextWrapping = TextWrapping.Wrap,
        FontSize = (double)Application.Current.Resources["SizeSmall"],
        Foreground = Res("TextFaint"),
        Margin = new Thickness(0, 6, 0, 14),
    };

    private static Border Button(string text, Action act)
    {
        var button = new Border
        {
            Padding = new Thickness(12, 6, 12, 6),
            CornerRadius = new CornerRadius(8),
            Background = Res("Raised"),
            Cursor = Cursors.Hand,
            HorizontalAlignment = HorizontalAlignment.Left,
            Child = new TextBlock { Text = text, Foreground = Res("TextDim") },
        };
        button.MouseEnter += (_, _) => button.Opacity = 0.8;
        button.MouseLeave += (_, _) => button.Opacity = 1.0;
        button.MouseLeftButtonUp += (_, _) => act();
        return button;
    }

    // --- Channels -------------------------------------------------------------

    /// <summary>Seven at most: the six that ship and one more.</summary>
    private const int MaxChannels = 7;

    private UIElement BuildChannelsPage()
    {
        var page = new StackPanel();
        page.Children.Add(PageHeading("Channels"));
        page.Children.Add(Explanation(
            "Lanes ships with six channels and has room for one more. Rename any of them by "
            + "double-clicking its name in the mixer. Only a channel you added can be removed; its apps go "
            + "back to To be routed, and are left playing exactly as they were."));
        page.Children.Add(_channelList);

        _newChannel.MaxLength = 24;
        _newChannel.Width = 200;
        _newChannel.Height = 30;
        _newChannel.Padding = new Thickness(6, 0, 6, 0);
        _newChannel.VerticalContentAlignment = VerticalAlignment.Center;
        _newChannel.Background = Res("Raised");
        _newChannel.Foreground = Res("Text");
        _newChannel.CaretBrush = Res("Text");
        _newChannel.BorderBrush = Res("LineStrong");
        _newChannel.BorderThickness = new Thickness(1);
        _newChannel.KeyDown += (_, e) =>
        {
            if (e.Key == Key.Enter)
            {
                AddChannel();
                e.Handled = true;
            }
        };

        var add = Button("Add channel", AddChannel);
        add.Margin = new Thickness(8, 0, 0, 0);
        _addChannelRow.Children.Add(_newChannel);
        _addChannelRow.Children.Add(add);
        page.Children.Add(_addChannelRow);

        _channelsFull.Text = "All seven channels are in use. Remove the added one to add a different one.";
        _channelsFull.FontSize = (double)Application.Current.Resources["SizeSmall"];
        _channelsFull.Foreground = Res("TextGhost");
        _channelsFull.Margin = new Thickness(0, 14, 0, 0);
        page.Children.Add(_channelsFull);

        // Not subscribed to the client here: the pages are built before the
        // client exists. The constructor's refusal handler fills this in.
        _channelProblem.FontSize = (double)Application.Current.Resources["SizeSmall"];
        _channelProblem.Foreground = Res("Warn");
        _channelProblem.TextWrapping = TextWrapping.Wrap;
        _channelProblem.Margin = new Thickness(0, 10, 0, 0);
        page.Children.Add(_channelProblem);

        return new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = page };
    }

    private void AddChannel()
    {
        var name = _newChannel.Text.Trim();
        if (name.Length == 0)
        {
            return;
        }
        _channelProblem.Text = "";
        _client.AddChannel(name);
        _newChannel.Text = "";
    }

    private void RefreshChannels()
    {
        _channelList.Children.Clear();

        foreach (var channel in _channels)
        {
            var row = new DockPanel { LastChildFill = true, Margin = new Thickness(0, 0, 0, 6), MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Left };

            if (channel.Removable)
            {
                // Two clicks, as deleting a profile is: a channel's rules go
                // with it, and they took some doing.
                var armed = false;
                Border? remove = null;
                remove = Button("Remove", () =>
                {
                    if (!armed)
                    {
                        armed = true;
                        ((TextBlock)remove!.Child).Text = "Remove?";
                        ((TextBlock)remove.Child).Foreground = Res("Warn");
                        remove.Background = Res("WarnGround");
                        return;
                    }
                    _client.RemoveChannel(channel.Id);
                });
                remove.MouseLeave += (_, _) =>
                {
                    armed = false;
                    ((TextBlock)remove.Child).Text = "Remove";
                    ((TextBlock)remove.Child).Foreground = Res("TextDim");
                    remove.Background = Res("Raised");
                };
                DockPanel.SetDock(remove, Dock.Right);
                row.Children.Add(remove);
            }

            var label = new TextBlock { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 16, 0) };
            label.Inlines.Add(new System.Windows.Documents.Run(channel.Name) { Foreground = Res("Text") });
            label.Inlines.Add(new System.Windows.Documents.Run(
                channel.IsInput ? "   input" : channel.Removable ? "   added" : "")
            {
                Foreground = Res("TextGhost"),
                FontSize = (double)Application.Current.Resources["SizeSmall"],
            });
            row.Children.Add(label);
            _channelList.Children.Add(row);
        }

        var full = _channels.Count >= MaxChannels;
        _addChannelRow.Visibility = full ? Visibility.Collapsed : Visibility.Visible;
        _channelsFull.Visibility = full ? Visibility.Visible : Visibility.Collapsed;
    }

    // --- Ignored apps ---------------------------------------------------------

    private UIElement BuildIgnoredPage()
    {
        var page = new StackPanel();
        page.Children.Add(PageHeading("Ignored apps"));
        page.Children.Add(Explanation(
            "Lanes never manages or shows these. Their audio is left exactly where it was when they "
            + "were ignored. Right-click an app in the mixer to ignore it."));
        page.Children.Add(_ignoredList);
        return new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = page };
    }

    private void RefreshIgnored()
    {
        _ignoredList.Children.Clear();

        if (_ignored.Count == 0)
        {
            _ignoredList.Children.Add(new TextBlock { Text = "Nothing is ignored.", Foreground = Res("TextGhost") });
            return;
        }

        foreach (var executable in _ignored)
        {
            var row = new DockPanel { LastChildFill = true, Margin = new Thickness(0, 0, 0, 6), MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Left };
            var stop = Button("Stop ignoring", () => _client.UnignoreApp(executable));
            DockPanel.SetDock(stop, Dock.Right);
            row.Children.Add(stop);
            row.Children.Add(new TextBlock
            {
                Text = executable,
                VerticalAlignment = VerticalAlignment.Center,
                Foreground = Res("Text"),
                Margin = new Thickness(0, 0, 16, 0),
            });
            _ignoredList.Children.Add(row);
        }
    }

    // --- Notifications ------------------------------------------------------------

    private UIElement BuildGeneralPage()
    {
        var page = new StackPanel();

        page.Children.Add(PageHeading("Appearance"));
        page.Children.Add(Explanation(
            "Lanes follows Windows' own light or dark setting unless you choose one here. Open windows "
            + "redraw in the new colours straight away."));
        foreach (var (value, label) in new[] { ("system", "Follow Windows"), ("dark", "Dark"), ("light", "Light") })
        {
            var option = new Border
            {
                Padding = new Thickness(14, 7, 14, 7),
                Margin = new Thickness(0, 0, 6, 0),
                CornerRadius = new CornerRadius(8),
                Cursor = Cursors.Hand,
                Tag = value,
                Child = new TextBlock { Text = label, FontSize = (double)Application.Current.Resources["SizeBody"] },
            };
            // Only sent. The core's reply carries the new setting back, and
            // that is what redraws - so a refused or lost command cannot leave
            // the window in colours the core does not know about. Choosing
            // locally first would also rebuild this window, and dispose its
            // connection, before the command had left it.
            option.MouseLeftButtonUp += (_, _) => _client.SetTheme(value);
            _themeChoice.Children.Add(option);
        }
        page.Children.Add(_themeChoice);
        RefreshTheme();

        page.Children.Add(PageHeading("Restore Windows audio"));
        page.Children.Add(Explanation(
            "Put everything back as it was before Lanes: every app at 100%, unmuted, on the default device, "
            + "and Windows' default device and volume as they were. Apps that are not running are reset the "
            + "next time they play. Lanes then pauses, so your channels are not put straight back, until you "
            + "resume it. Your channels, rules and profiles are kept."));
        var restoreRow = new StackPanel { Orientation = Orientation.Horizontal };
        var restore = new Button
        {
            Content = "Restore Windows audio…",
            Style = (Style)Application.Current.Resources["QuietButton"],
            Padding = new Thickness(14, 7, 14, 7),
        };
        var armed = false;
        restore.Click += (_, _) =>
        {
            // Two clicks, as with removing a channel: this undoes everything
            // Lanes has done to every app at once.
            if (!armed)
            {
                armed = true;
                restore.Content = "Click again to restore and pause";
                restore.Foreground = Res("Warn");
                return;
            }
            armed = false;
            restore.Content = "Restore Windows audio…";
            restore.Foreground = Res("TextDim");
            _client.RestoreWindows();
        };
        restore.MouseLeave += (_, _) =>
        {
            armed = false;
            restore.Content = "Restore Windows audio…";
            restore.Foreground = Res("TextDim");
        };
        restoreRow.Children.Add(restore);
        _resume.Style = (Style)Application.Current.Resources["QuietButton"];
        _resume.Padding = new Thickness(14, 7, 14, 7);
        _resume.Margin = new Thickness(8, 0, 0, 0);
        _resume.Click += (_, _) => _client.Resume();
        restoreRow.Children.Add(_resume);
        page.Children.Add(restoreRow);
        _restoreState.Foreground = Res("Warn");
        _restoreState.Margin = new Thickness(0, 8, 0, 26);
        page.Children.Add(_restoreState);

        page.Children.Add(PageHeading("Notifications"));
        page.Children.Add(Explanation(
            "When an app that is in no channel starts playing, show a small notice in the corner with a "
            + "button for each channel. It never takes focus, and it stays quiet while a game, video or "
            + "presentation is full-screen - the \"To be routed\" count turns amber instead."));

        var row = new StackPanel { Orientation = Orientation.Horizontal };
        _notify.Toggled += on => _client.SetNotifyNewApps(on);
        row.Children.Add(_notify);
        row.Children.Add(new TextBlock
        {
            Text = "Tell me when an app with no channel starts playing",
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = Res("TextDim"),
            Margin = new Thickness(12, 0, 0, 0),
        });
        page.Children.Add(row);
        return page;
    }

    /// <summary>Say whether Lanes is paused, and offer the way out when it is.</summary>
    private void ShowPaused(bool paused)
    {
        _paused = paused;
        _resume.Visibility = paused ? Visibility.Visible : Visibility.Collapsed;
        _restoreState.Text = paused
            ? "Lanes is paused: Windows audio is as it was before Lanes, and your channels are not being applied."
            : "";
    }

    /// <summary>Mark the theme option in force, in the same idiom as the tabs.</summary>
    private void RefreshTheme()
    {
        foreach (Border option in _themeChoice.Children)
        {
            var chosen = (string)option.Tag == Themes.Choice;
            option.Background = Res(chosen ? "Raised" : "Ground");
            option.BorderBrush = Res(chosen ? "Raised" : "Line");
            option.BorderThickness = new Thickness(1);
            ((TextBlock)option.Child).Foreground = Res(chosen ? "Text" : "TextFaint");
        }
    }

    // --- Backup ----------------------------------------------------------------------

    private UIElement BuildBackupPage()
    {
        var page = new StackPanel();
        page.Children.Add(PageHeading("Backup"));
        page.Children.Add(Explanation(
            "Export saves your channels, app rules, ignored apps, profiles, hotkeys and device order to a "
            + "file. Import replaces them with a file's. Your API token and where the window sits stay "
            + "as they are, and so does Start with Windows. Before importing, Lanes snapshots your audio "
            + "and keeps your current settings beside the new ones as config.json.before-import."));

        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 12) };
        buttons.Children.Add(Button("Export settings…", () =>
        {
            _backupMessage.Text = "";
            _client.ExportConfig();
        }));
        var import = Button("Import settings…", ImportFromFile);
        import.Margin = new Thickness(8, 0, 0, 0);
        buttons.Children.Add(import);
        page.Children.Add(buttons);

        _backupMessage.TextWrapping = TextWrapping.Wrap;
        _backupMessage.FontSize = (double)Application.Current.Resources["SizeSmall"];
        page.Children.Add(_backupMessage);
        return page;
    }

    private void SaveExport(System.Text.Json.JsonElement config)
    {
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Title = "Export Lanes settings",
            FileName = $"Lanes settings {DateTime.Now:yyyy-MM-dd}.json",
            Filter = "Lanes settings (*.json)|*.json",
            DefaultExt = ".json",
        };
        if (dialog.ShowDialog(this) != true)
        {
            return;
        }

        try
        {
            var json = System.Text.Json.JsonSerializer.Serialize(
                config, new System.Text.Json.JsonSerializerOptions { WriteIndented = true });
            System.IO.File.WriteAllText(dialog.FileName, json, new System.Text.UTF8Encoding(false));
            _backupMessage.Text = $"Saved to {dialog.FileName}.";
            _backupMessage.Foreground = Res("TextDim");
        }
        catch (Exception ex)
        {
            _backupMessage.Text = $"Could not save: {ex.Message}";
            _backupMessage.Foreground = Res("Warn");
        }
    }

    private void ImportFromFile()
    {
        var dialog = new Microsoft.Win32.OpenFileDialog
        {
            Title = "Import Lanes settings",
            Filter = "Lanes settings (*.json)|*.json|All files (*.*)|*.*",
        };
        if (dialog.ShowDialog(this) != true)
        {
            return;
        }

        try
        {
            using var document = System.Text.Json.JsonDocument.Parse(System.IO.File.ReadAllText(dialog.FileName));
            _client.ImportConfig(document.RootElement.Clone());
            _backupMessage.Text =
                "Imported. Your previous settings are kept as config.json.before-import, and the audio as it "
                + "was is in a snapshot that Restore can return to.";
            _backupMessage.Foreground = Res("TextDim");
        }
        catch (Exception ex)
        {
            _backupMessage.Text = $"That file could not be read: {ex.Message}";
            _backupMessage.Foreground = Res("Warn");
        }
    }

    // --- Diagnostics ---------------------------------------------------------------------

    private UIElement BuildDiagnosticsPage()
    {
        var page = new StackPanel();
        page.Children.Add(PageHeading("Diagnostics"));
        page.Children.Add(Explanation(
            "What Lanes can see, what it makes of each app, and the last things it changed - the first "
            + "place to look when something is not where you expected."));

        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 14) };
        buttons.Children.Add(Button("Refresh", () => _client.GetDiagnostics()));
        page.Children.Add(buttons);
        page.Children.Add(_diagnostics);

        return new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = page };
    }

    private void ShowDiagnostics(Diagnostics d)
    {
        _diagnostics.Children.Clear();

        TextBlock Line(string text, string brush = "TextDim", bool mono = false) => new()
        {
            Text = text,
            Foreground = Res(brush),
            FontSize = (double)Application.Current.Resources["SizeSmall"],
            FontFamily = mono ? new FontFamily("Cascadia Mono, Consolas") : FontFamily,
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 3),
        };

        TextBlock Section(string text) => new()
        {
            Text = text,
            FontWeight = FontWeights.SemiBold,
            Foreground = Res("Text"),
            Margin = new Thickness(0, 14, 0, 6),
        };

        _diagnostics.Children.Add(Line(d.RoutingAvailable
            ? "Per-app device routing: available."
            : "Per-app device routing: UNAVAILABLE on this Windows build. Volumes still work.",
            d.RoutingAvailable ? "TextDim" : "Warn"));

        var folder = new DockPanel { LastChildFill = true, Margin = new Thickness(0, 4, 0, 0) };
        var open = Button("Open folder", () =>
        {
            try
            {
                System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo
                {
                    FileName = d.StateFolder,
                    UseShellExecute = true,
                });
            }
            catch (Exception)
            {
                // Nothing useful to say beyond the path, which is on screen.
            }
        });
        DockPanel.SetDock(open, Dock.Right);
        folder.Children.Add(open);
        folder.Children.Add(Line(
            $"Settings, snapshots and logs: {d.StateFolder}{(d.Portable ? "  (portable copy)" : "")}",
            mono: false));
        _diagnostics.Children.Add(folder);

        _diagnostics.Children.Add(Section($"Audio sessions ({d.Sessions.Count})"));
        foreach (var s in d.Sessions.OrderBy(s => s.Executable, StringComparer.OrdinalIgnoreCase))
        {
            var where = s.Ignored ? "ignored"
                : s.Channel is { } channel ? $"{channel} (rule: {s.Rule})"
                : "to be routed";
            _diagnostics.Children.Add(Line(
                $"{s.Executable,-28} pid {s.ProcessId,-6} {s.State,-8} {s.Volume * 100,4:0}%{(s.Muted ? " muted" : "      ")}  {where}",
                s.Channel is null && !s.Ignored ? "Warn" : "TextDim",
                mono: true));
        }

        _diagnostics.Children.Add(Section("Recent changes"));
        if (d.Recent.Count == 0)
        {
            _diagnostics.Children.Add(Line("None today or yesterday.", "TextGhost"));
        }
        foreach (var entry in d.Recent.Reverse())
        {
            string Field(string name) =>
                entry.TryGetProperty(name, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.String
                    ? v.GetString() ?? ""
                    : "";
            var at = Field("at");
            var time = at.Length >= 19 ? at.Substring(11, 8) : at;
            var ok = !entry.TryGetProperty("ok", out var okValue) || okValue.ValueKind != System.Text.Json.JsonValueKind.False;
            _diagnostics.Children.Add(Line(
                $"{time}  {Field("target")}  {Field("field")}: {Field("old")} → {Field("new")}  ({Field("reason")}){(ok ? "" : "  FAILED")}",
                ok ? "TextDim" : "Warn",
                mono: true));
        }
    }

    // --- Hotkeys -----------------------------------------------------------------------

    private UIElement BuildHotkeysPage()
    {
        var heading = new TextBlock
        {
            Text = "Hotkeys",
            FontSize = (double)Application.Current.Resources["SizeChannel"],
            FontWeight = FontWeights.SemiBold,
            Foreground = Res("Text"),
        };

        var explanation = new TextBlock
        {
            Text = "They work anywhere in Windows, even with the mixer closed. Each needs Ctrl, Alt or Win "
                 + "(or one of F13-F24 alone), so it cannot take an ordinary key away from other programs.",
            TextWrapping = TextWrapping.Wrap,
            FontSize = (double)Application.Current.Resources["SizeSmall"],
            Foreground = Res("TextFaint"),
            Margin = new Thickness(0, 6, 0, 14),
        };

        _problem.TextWrapping = TextWrapping.Wrap;
        _problem.FontSize = (double)Application.Current.Resources["SizeSmall"];
        _problem.Foreground = Res("Warn");
        _problem.Margin = new Thickness(0, 0, 0, 10);

        var add = new Border
        {
            Padding = new Thickness(12, 6, 12, 6),
            CornerRadius = new CornerRadius(8),
            Background = Res("Raised"),
            Cursor = Cursors.Hand,
            HorizontalAlignment = HorizontalAlignment.Left,
            Margin = new Thickness(0, 10, 0, 0),
            Child = new TextBlock { Text = "Add a hotkey", Foreground = Res("TextDim") },
        };
        add.MouseEnter += (_, _) => add.Opacity = 0.8;
        add.MouseLeave += (_, _) => add.Opacity = 1.0;
        add.MouseLeftButtonUp += (_, _) =>
        {
            var row = new Row();
            _model.Add(row);
            _rows.Children.Add(BuildRow(row));
            RefreshStatus();
        };

        var list = new StackPanel();
        list.Children.Add(_rows);
        list.Children.Add(add);

        var top = new StackPanel();
        top.Children.Add(heading);
        top.Children.Add(explanation);
        top.Children.Add(_problem);
        DockPanel.SetDock(top, Dock.Top);

        var layout = new DockPanel { LastChildFill = true };
        layout.Children.Add(top);
        layout.Children.Add(new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Content = list,
        });
        return layout;
    }

    /// <summary>Build the rows once, from what the core holds.</summary>
    /// <remarks>
    /// Once only: rebuilding on later pushes would pull a row out from under a
    /// choice being made. Later pushes update the status column and the lists
    /// of channels and profiles in place.
    /// </remarks>
    private void BuildRows()
    {
        _rows.Children.Clear();
        _model.Clear();

        foreach (var hotkey in _live)
        {
            var row = new Row
            {
                Keys = hotkey.Keys,
                Kind = hotkey.Action.Kind,
                Target = hotkey.Action.Channel ?? hotkey.Action.Name,
            };
            _model.Add(row);
            _rows.Children.Add(BuildRow(row));
        }
    }

    private Grid BuildRow(Row row)
    {
        var grid = new Grid { Margin = new Thickness(0, 0, 0, 8) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(150) });   // keys
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(8) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(180) });   // action
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(8) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(140) });   // target
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) }); // status
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });       // remove

        var keys = new KeyCapture { Keys = row.Keys };
        keys.Captured += captured =>
        {
            row.Keys = captured;
            Changed();
        };
        Grid.SetColumn(keys, 0);
        grid.Children.Add(keys);

        var action = new ChoiceButton { Placeholder = "Choose an action" };
        action.SetChoices(Actions.Select(a => (a.Kind, a.Label)).ToList(), row.Kind);
        action.Chosen += kind =>
        {
            if (Needs(kind) != Needs(row.Kind))
            {
                row.Target = null;
            }
            row.Kind = kind;
            RefreshTarget(row);
            Changed();
        };
        Grid.SetColumn(action, 2);
        grid.Children.Add(action);

        row.TargetChoice.Chosen += target =>
        {
            row.Target = target;
            Changed();
        };
        Grid.SetColumn(row.TargetChoice, 4);
        grid.Children.Add(row.TargetChoice);
        RefreshTarget(row);

        row.Status.VerticalAlignment = VerticalAlignment.Center;
        row.Status.Margin = new Thickness(12, 0, 8, 0);
        row.Status.FontSize = (double)Application.Current.Resources["SizeSmall"];
        row.Status.TextTrimming = TextTrimming.CharacterEllipsis;
        Grid.SetColumn(row.Status, 5);
        grid.Children.Add(row.Status);

        var remove = new Border
        {
            Padding = new Thickness(8, 4, 8, 4),
            CornerRadius = new CornerRadius(6),
            Background = Brushes.Transparent,
            Cursor = Cursors.Hand,
            VerticalAlignment = VerticalAlignment.Center,
            ToolTip = "Remove this hotkey",
            Child = new TextBlock
            {
                Text = "Remove",
                FontSize = (double)Application.Current.Resources["SizeSmall"],
                Foreground = Res("TextFaint"),
            },
        };
        remove.MouseEnter += (_, _) => remove.Background = Res("Hover");
        remove.MouseLeave += (_, _) => remove.Background = Brushes.Transparent;
        remove.MouseLeftButtonUp += (_, _) =>
        {
            _model.Remove(row);
            _rows.Children.Remove(grid);
            Changed();
        };
        Grid.SetColumn(remove, 6);
        grid.Children.Add(remove);

        return grid;
    }

    private static Target Needs(string? kind) =>
        Actions.FirstOrDefault(a => a.Kind == kind).Needs;

    /// <summary>Offer the right kind of target for the row's action, or none.</summary>
    private void RefreshTarget(Row row)
    {
        var needs = Needs(row.Kind);
        row.TargetChoice.Visibility = needs == Target.None ? Visibility.Hidden : Visibility.Visible;

        IReadOnlyList<(string, string)> choices = needs switch
        {
            Target.Channel => _channels.Select(c => (c.Id, c.Name)).ToList(),
            // The microphone signal is never routed, so it has no output to cycle.
            Target.OutputChannel => _channels.Where(c => !c.IsInput).Select(c => (c.Id, c.Name)).ToList(),
            Target.Profile => _profiles.Select(p => (p, p)).ToList(),
            _ => [],
        };

        row.TargetChoice.Placeholder = needs switch
        {
            Target.Profile when _profiles.Count == 0 => "No profiles saved",
            Target.Profile => "Choose a profile",
            _ => "Choose a channel",
        };
        row.TargetChoice.SetChoices(choices, row.Target);
    }

    private void RefreshTargets()
    {
        foreach (var row in _model)
        {
            RefreshTarget(row);
        }
    }

    private static bool Complete(Row row) =>
        row.Keys is not null
        && row.Kind is not null
        && (Needs(row.Kind) == Target.None || row.Target is not null);

    /// <summary>Send every complete row. Incomplete ones wait here until they are.</summary>
    private void Changed()
    {
        _problem.Text = "";

        var bindings = _model.Where(Complete).Select(row =>
        {
            var needs = Needs(row.Kind);
            var action = needs switch
            {
                Target.Channel or Target.OutputChannel => new HotkeyAction(row.Kind!, Channel: row.Target),
                Target.Profile => new HotkeyAction(row.Kind!, Name: row.Target),
                _ => new HotkeyAction(row.Kind!),
            };
            return (row.Keys!, action);
        }).ToList();

        _saving = true;
        _client.SetHotkeys(bindings);
        RefreshStatus();
    }

    /// <summary>
    /// Say, for each row, whether it is live - from the core's answer, not from
    /// what this window sent.
    /// </summary>
    private void RefreshStatus()
    {
        foreach (var row in _model)
        {
            var (text, key, why) = Status(row);
            row.Status.Text = text;
            row.Status.Foreground = Res(key);
            row.Status.ToolTip = why ?? text;
        }
    }

    private (string Text, string Brush, string? Why) Status(Row row)
    {
        if (!Complete(row))
        {
            return ("Incomplete", "TextGhost", "Needs keys, an action, and a channel or profile if the action names one");
        }

        // A second row with the same keys is never the live one, whatever the
        // core holds. Matching on keys alone would show a refused duplicate as
        // "Active", because the row above it is.
        var first = _model.First(r => r.Keys == row.Keys && Complete(r));
        if (!ReferenceEquals(first, row))
        {
            return ("Keys used above", "Warn", "One key combination can do one thing");
        }

        // Live only if the core holds this exact binding: same keys AND the
        // same action. Otherwise it was refused, or is still on its way.
        var live = _live.FirstOrDefault(h =>
            h.Keys == row.Keys
            && h.Action.Kind == row.Kind
            && (h.Action.Channel ?? h.Action.Name) == row.Target);
        if (live is null)
        {
            return _saving
                ? ("Saving…", "TextGhost", null)
                : ("Not saved", "Warn", "Lanes refused this list - the reason is shown above");
        }

        return live.Status switch
        {
            "active" => ("Active", "TextDim", "Registered with Windows and working"),
            "in_use" => ("In use elsewhere", "Warn",
                "Another program has registered these keys, so pressing them does nothing here. Choose other keys, or close that program."),
            "failed" => ("Refused", "Warn", live.Problem ?? "Windows refused it"),
            _ => ("Waiting for Lanes", "TextGhost", "Lanes has not registered it yet"),
        };
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
