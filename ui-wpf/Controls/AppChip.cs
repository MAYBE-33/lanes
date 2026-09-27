using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;

namespace Lanes.Ui.Controls;

/// <summary>
/// One application, in a channel's list or in the "To be routed" pool.
/// </summary>
/// <remarks>
/// <para>
/// <b>The idle state is carried by a dot and the text colour, not by fading the
/// whole chip.</b> Fading the entire control takes the box down with the text,
/// and a chip you cannot read is a chip you cannot drag with any confidence
/// about what you are dragging.
/// </para>
/// <para>
/// So the box keeps its contrast in both states and only the two things that
/// are actually reporting status change: a dot that lights in the channel's
/// colour while the application is making sound, and text that steps down one
/// level when it is not. Both states stay readable, which is the point.
/// </para>
/// <para>
/// <b>Updated in place, never rebuilt.</b> The playing flag flips constantly,
/// and rebuilding a chip somebody might be dragging would drop the drag - see
/// <c>docs/window.md</c>, "Update in place".
/// </para>
/// </remarks>
public sealed class AppChip : Border
{
    private const double DotSize = 6;

    private readonly System.Windows.Shapes.Ellipse _dot = new()
    {
        Width = DotSize,
        Height = DotSize,
        VerticalAlignment = VerticalAlignment.Center,
        Margin = new Thickness(0, 0, 8, 0),
    };

    /// <summary>
    /// The application's own icon. Collapsed when it has none, rather than
    /// leaving a gap - see <see cref="AppIcons"/>.
    /// </summary>
    private readonly Image _icon = new()
    {
        Width = 16,
        Height = 16,
        VerticalAlignment = VerticalAlignment.Center,
        Margin = new Thickness(0, 0, 7, 0),
    };

    private readonly TextBlock _label = new()
    {
        VerticalAlignment = VerticalAlignment.Center,
        TextTrimming = TextTrimming.CharacterEllipsis,
    };

    /// <summary>
    /// An amber mark for an application that would not keep the output device
    /// its channel gave it, so Lanes has stopped trying. The tooltip says what
    /// to do about it. See <c>engine::REFUSED</c> in the core.
    /// </summary>
    private readonly TextBlock _ownDevice = new()
    {
        Text = "!",
        FontWeight = FontWeights.Bold,
        VerticalAlignment = VerticalAlignment.Center,
        Margin = new Thickness(6, 0, 0, 0),
        Visibility = Visibility.Collapsed,
    };

    /// <summary>The executable this chip stands for.</summary>
    public string Executable { get; private set; } = "";

    /// <summary>What the right-click menu can do. Set once by the window.</summary>
    /// <remarks>
    /// Static rather than wired chip by chip: chips are created in two places
    /// (the strips and the pool), and threading a set of callbacks through
    /// both, and through every reflow that moves them, would be the same
    /// wiring three times. One window per process makes this safe.
    /// </remarks>
    public static IAppCommands? Commands { get; set; }

    private SessionState? _session;

    // --- Selection -------------------------------------------------------------
    //
    // Ctrl-click chips to gather several, then drag any one of them to move
    // them all. Static, like Commands, because a selection spans the strips and
    // the pool, and there is one mixer per process.

    private static readonly HashSet<string> Selection = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>Raised when the selection changes, so every chip can repaint.</summary>
    private static event Action? SelectionChanged;

    /// <summary>Forget the selection: a plain click elsewhere, Esc, or after a move.</summary>
    public static void ClearSelection()
    {
        if (Selection.Count == 0)
        {
            return;
        }

        Selection.Clear();
        SelectionChanged?.Invoke();
    }

    /// <summary>What a drag carries: executables, one per line.</summary>
    /// <remarks>
    /// A plain string so every existing drop target keeps accepting it; a
    /// single app is simply a list of one. <see cref="Dragged"/> reads it.
    /// </remarks>
    private string Payload() =>
        Selection.Contains(Executable) && Selection.Count > 1
            ? string.Join('\n', Selection)
            : Executable;

    /// <summary>The executables a drop carries, whichever chip it came from.</summary>
    public static IReadOnlyList<string> Dragged(IDataObject data) =>
        data.GetData(DataFormats.StringFormat) is string text
            ? text.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            : [];

    private bool Selected => Selection.Contains(Executable);

    private void PaintBorder() =>
        BorderBrush = (Brush)Application.Current.Resources[
            Selected ? "TextDim" : IsMouseOver ? "ChipLineHot" : "ChipLine"];

    public AppChip()
    {
        Height = 28;
        CornerRadius = new CornerRadius(8);
        Background = (Brush)Application.Current.Resources["Chip"];
        BorderBrush = (Brush)Application.Current.Resources["ChipLine"];
        BorderThickness = new Thickness(1);
        Margin = new Thickness(0, 0, 0, 4);
        Padding = new Thickness(9, 0, 9, 0);
        Cursor = Cursors.Hand;
        SnapsToDevicePixels = true;

        var row = new StackPanel { Orientation = Orientation.Horizontal };
        row.Children.Add(_dot);
        row.Children.Add(_icon);
        row.Children.Add(_label);
        row.Children.Add(_ownDevice);
        RenderOptions.SetBitmapScalingMode(_icon, BitmapScalingMode.HighQuality);
        Child = row;

        MouseEnter += (_, _) => PaintBorder();
        MouseLeave += (_, _) => PaintBorder();

        // Subscribed only while on screen, so a chip that has been thrown away
        // is not kept alive by a static event.
        Loaded += (_, _) => { SelectionChanged += PaintBorder; PaintBorder(); };
        Unloaded += (_, _) => SelectionChanged -= PaintBorder;

        // Ctrl-click adds or removes this chip; a plain click lets the
        // selection go. Handled, so the window's own "click elsewhere clears
        // it" does not undo the click that made it.
        MouseLeftButtonDown += (_, e) =>
        {
            if (Keyboard.Modifiers.HasFlag(ModifierKeys.Control) && Executable.Length > 0)
            {
                if (!Selection.Remove(Executable))
                {
                    Selection.Add(Executable);
                }
                SelectionChanged?.Invoke();
                e.Handled = true;
            }
            else if (!Selected)
            {
                ClearSelection();
            }
        };

        // Right-click: "always assign to…", "ignore this app", and this app's
        // level within its channel.
        MouseRightButtonUp += (_, e) =>
        {
            e.Handled = true;
            OpenMenu();
        };

        // The drag starts on a move with the button held rather than on the
        // press, so a plain click does not begin one.
        MouseMove += (_, e) =>
        {
            if (e.LeftButton == MouseButtonState.Pressed && Executable.Length > 0)
            {
                // Dragging a chip outside the selection moves just that chip.
                if (!Selected)
                {
                    ClearSelection();
                }

                var moved = DragDrop.DoDragDrop(this, Payload(), DragDropEffects.Move);
                if (moved != DragDropEffects.None)
                {
                    ClearSelection();
                }
            }
        };
    }

    /// <summary>
    /// Show this session.
    /// </summary>
    /// <param name="accent">
    /// The owning channel's colour for the playing dot, or null in the pool,
    /// where the application belongs to no channel and there is no colour that
    /// would be honest.
    /// </param>
    public void Update(SessionState session, Brush? accent)
    {
        Executable = session.Executable;
        _session = session;

        // A trimmed app says so, because otherwise nothing on screen explains
        // why it is quieter than its channel-mates.
        _label.Text = session.Trim is { } trim
            ? $"{session.Executable}  ·  {Math.Round(trim * 100)}%"
            : session.Executable;

        _dot.Fill = session.Playing
            ? accent ?? (Brush)Application.Current.Resources["Warn"]
            : (Brush)Application.Current.Resources["DotIdle"];

        _label.Foreground = (Brush)Application.Current.Resources[session.Playing ? "Text" : "TextDim"];

        // Assigned-but-idle steps down with the text. Only when the source
        // changes, so a playing flag flipping does not reassign the image.
        var icon = AppIcons.For(session.Path);
        if (!ReferenceEquals(_icon.Source, icon))
        {
            _icon.Source = icon;
        }
        _icon.Visibility = icon is null ? Visibility.Collapsed : Visibility.Visible;
        _icon.Opacity = session.Playing ? 1.0 : 0.6;

        _ownDevice.Foreground = (Brush)Application.Current.Resources["Warn"];
        _ownDevice.Visibility = session.RoutingRefused ? Visibility.Visible : Visibility.Collapsed;

        ToolTip = Describe(session);
    }

    /// <summary>The steps offered for an app's level within its channel.</summary>
    private static readonly double[] TrimSteps = [1.0, 0.8, 0.6, 0.4, 0.2];

    private void OpenMenu()
    {
        if (Commands is not { } commands || _session is not { } session)
        {
            return;
        }

        var items = new StackPanel();
        var popup = new Popup
        {
            PlacementTarget = this,
            Placement = PlacementMode.Bottom,
            VerticalOffset = 4,
            StaysOpen = false,
            AllowsTransparency = true,
            PopupAnimation = PopupAnimation.Fade,
            Child = new Border
            {
                Background = Res("PopupGround"),
                BorderBrush = Res("LineStrong"),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(9),
                Padding = new Thickness(5),
                MinWidth = 210,
                Child = items,
            },
        };

        if (session.Channel is not null)
        {
            items.Children.Add(Heading("Volume within its channel"));

            var steps = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(4, 2, 4, 6) };
            var current = session.Trim ?? 1.0;
            foreach (var step in TrimSteps)
            {
                var chosen = Math.Abs(step - current) < 0.01;
                var button = new Border
                {
                    Padding = new Thickness(7, 4, 7, 4),
                    Margin = new Thickness(0, 0, 4, 0),
                    CornerRadius = new CornerRadius(6),
                    Background = Res(chosen ? "LineStrong" : "Raised"),
                    Cursor = Cursors.Hand,
                    Child = new TextBlock
                    {
                        Text = $"{step * 100:0}%",
                        FontSize = Size("SizeSmall"),
                        FontWeight = chosen ? FontWeights.Bold : FontWeights.Normal,
                        Foreground = Res(chosen ? "Text" : "TextDim"),
                    },
                };
                var value = step;
                button.MouseLeftButtonUp += (_, _) =>
                {
                    popup.IsOpen = false;
                    commands.SetTrim(session.Executable, value >= 0.999 ? null : value);
                };
                steps.Children.Add(button);
            }
            items.Children.Add(steps);

            items.Children.Add(Row("Remove from channel", () =>
            {
                popup.IsOpen = false;
                commands.Unassign(session.Executable);
            }));
        }
        else
        {
            // "Always assign to…": the same persistent rule a drag creates.
            items.Children.Add(Heading("Put in"));
            foreach (var (id, name) in commands.Channels)
            {
                items.Children.Add(Row(name, () =>
                {
                    popup.IsOpen = false;
                    commands.Assign(session.Executable, id);
                }));
            }
        }

        items.Children.Add(new Border { Height = 1, Background = Res("LineStrong"), Margin = new Thickness(6, 4, 6, 4) });
        items.Children.Add(Row("Ignore this app", () =>
        {
            popup.IsOpen = false;
            commands.Ignore(session.Executable);
        }, "Never manage or show it. Undo in Settings > Ignored apps."));

        popup.IsOpen = true;
    }

    private static TextBlock Heading(string text) => new()
    {
        Text = text,
        FontSize = Size("SizeTiny"),
        Foreground = Res("TextFaint"),
        Margin = new Thickness(8, 4, 8, 4),
    };

    private static Border Row(string text, Action act, string? tooltip = null)
    {
        var row = new Border
        {
            Height = 28,
            CornerRadius = new CornerRadius(6),
            Background = Brushes.Transparent,
            Cursor = Cursors.Hand,
            ToolTip = tooltip,
            Child = new TextBlock
            {
                Text = text,
                Margin = new Thickness(8, 0, 8, 0),
                VerticalAlignment = VerticalAlignment.Center,
                Foreground = Res("TextDim"),
            },
        };
        row.MouseEnter += (_, _) => row.Background = Res("LineStrong");
        row.MouseLeave += (_, _) => row.Background = Brushes.Transparent;
        row.MouseLeftButtonUp += (_, _) => act();
        return row;
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];

    private static double Size(string key) => (double)Application.Current.Resources[key];

    private static string Describe(SessionState session)
    {
        var state = session.Playing ? "playing" : "idle";
        var line = session.SessionCount > 1
            ? $"{session.Executable} — {state}, {session.SessionCount} sessions"
            : $"{session.Executable} — {state}";

        return session.RoutingRefused
            ? line + "\nSomething keeps changing its output device back - the app itself, or another "
                + "program - so Lanes has stopped moving it. Its volume and mute still follow the channel. "
                + "Dragging it to a channel again gives it one more try."
            : line;
    }
}

/// <summary>What a channel's own menu asks the window to do.</summary>
public interface IChannelCommands
{
    /// <summary>The two channels the Game/Chat mix balances.</summary>
    (string A, string B) MixPair { get; }

    /// <summary>Channels that can be one end of the mix, in strip order: every
    /// playback channel except Master.</summary>
    IReadOnlyList<(string Id, string Name)> MixCandidates { get; }

    /// <summary>Balance these two. The mix returns to centre.</summary>
    void SetMixPair(string a, string b);

    /// <summary>Remove the channel (only the added one can be).</summary>
    void Remove(string channel);
}

/// <summary>What an app chip's menu asks the window to do.</summary>
public interface IAppCommands
{
    /// <summary>Playback channels, id and name, in strip order.</summary>
    IReadOnlyList<(string Id, string Name)> Channels { get; }

    void Assign(string executable, string channel);
    void Unassign(string executable);
    void Ignore(string executable);
    void SetTrim(string executable, double? trim);
}
