using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;

namespace Lanes.Ui.Controls;

/// <summary>
/// "Audio devices": the global device fallback order.
/// </summary>
/// <remarks>
/// <para>
/// <b>One list for the whole application, ordered like a BIOS boot order.</b>
/// When a channel's own device is unplugged, it goes to the first enabled,
/// plugged-in device here. Devices are moved up and down with the arrows;
/// <i>Disable</i> excludes one from fallback, greys it, and pins it to the
/// bottom under "Disabled". The core keeps the list in step with the hardware:
/// a device seen for the first time appears at the bottom of the enabled ones,
/// enabled, and an unplugged one keeps its place.
/// </para>
/// <para>
/// Global rather than per channel, because which device is next best is a fact
/// about the hardware on the desk, not about Game or Chat.
/// </para>
/// <para>
/// <b>The same shapes as the "To be routed" pool, deliberately.</b> A rail in
/// the wide layout, a button with an overlay in the narrow ones, collapsed by
/// default and opened only by the user - one collapse pattern in the window
/// rather than two, and nothing that expands itself.
/// </para>
/// <para>
/// <b>Arrows rather than drag and drop.</b> A boot order is the model, and
/// those are moved one step at a time. Arrows are also exact with a
/// mouse and cannot drop a device in the wrong place by a pixel.
/// </para>
/// </remarks>
public sealed class DevicePanel : Border
{
    private readonly Border _rail = new();
    private readonly DockPanel _expanded = new();
    private readonly StackPanel _rows = new();
    private readonly TextBlock _railLabel = new();
    private readonly Border _button = new();
    private readonly Popup _overlay = new();

    private bool _open;
    private LayoutMode _mode = LayoutMode.Full;
    private IReadOnlyList<PriorityState> _list = [];

    /// <summary>What the list was last drawn from; see <see cref="Update"/>.</summary>
    private string _signature = "";

    /// <summary>The whole order, as the user wants it: ids with enabled flags.</summary>
    public event Action<IReadOnlyList<(string Id, bool Enabled)>>? PriorityChanged;

    public DevicePanel()
    {
        Background = Brushes.Transparent;

        BuildRail();
        BuildExpanded();
        BuildButton();
        Child = _rail;
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];

    private static double Size(string key) => (double)Application.Current.Resources[key];

    private void BuildRail()
    {
        var stack = new StackPanel
        {
            Width = 46,
            HorizontalAlignment = HorizontalAlignment.Center,
            Margin = new Thickness(0, 14, 0, 14),
        };

        var toggle = RoundButton(
            Icons.Chevron(Icons.Direction.Right, Res("TextFaint")),
            "Device fallback order");
        toggle.MouseLeftButtonUp += (_, _) => Toggle();
        stack.Children.Add(toggle);

        var speaker = Icons.Speaker(false, Res("TextGhost"), 14);
        speaker.HorizontalAlignment = HorizontalAlignment.Center;
        speaker.Margin = new Thickness(0, 12, 0, 0);
        stack.Children.Add(speaker);

        // Rotated with a layout transform, so the box turns with the text.
        _railLabel.Text = "Audio devices";
        _railLabel.Foreground = Res("TextGhost");
        _railLabel.FontSize = Size("SizeTiny");
        _railLabel.LayoutTransform = new RotateTransform(90);
        _railLabel.Margin = new Thickness(0, 14, 0, 0);
        _railLabel.HorizontalAlignment = HorizontalAlignment.Center;
        stack.Children.Add(_railLabel);

        _rail.Background = Res("Card");
        _rail.BorderBrush = Res("Line");
        _rail.BorderThickness = new Thickness(1);
        _rail.CornerRadius = new CornerRadius(14);
        _rail.Child = stack;
    }

    private void BuildExpanded()
    {
        _expanded.Width = 268;
        _expanded.LastChildFill = true;

        var header = new DockPanel { LastChildFill = true, Height = 22 };
        var toggle = new Border
        {
            Cursor = Cursors.Hand,
            Background = Brushes.Transparent,
            VerticalAlignment = VerticalAlignment.Center,
            Padding = new Thickness(2, 4, 9, 4),
            Child = Icons.Chevron(Icons.Direction.Left, Res("TextFaint")),
        };
        toggle.MouseLeftButtonUp += (_, _) => Toggle();
        DockPanel.SetDock(toggle, Dock.Left);
        header.Children.Add(toggle);
        header.Children.Add(new TextBlock
        {
            Text = "Audio devices",
            FontSize = Size("SizeBody"),
            FontWeight = FontWeights.SemiBold,
            VerticalAlignment = VerticalAlignment.Center,
        });
        DockPanel.SetDock(header, Dock.Top);
        _expanded.Children.Add(header);

        var section = new TextBlock
        {
            Text = "Device fallback",
            Foreground = Res("TextDim"),
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 14, 0, 4),
        };
        DockPanel.SetDock(section, Dock.Top);
        _expanded.Children.Add(section);

        var help = new TextBlock
        {
            Text = "When a channel's device is unplugged, it uses the first available device in this list.",
            Foreground = Res("TextGhost"),
            FontSize = Size("SizeTiny"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 10),
        };
        DockPanel.SetDock(help, Dock.Top);
        _expanded.Children.Add(help);

        // Last, so it takes whatever height is left and scrolls within it.
        _expanded.Children.Add(new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            MaxHeight = 420,
            Content = _rows,
        });
    }

    private void BuildButton()
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal };
        var speaker = Icons.Speaker(false, Res("TextDim"), 13);
        speaker.VerticalAlignment = VerticalAlignment.Center;
        speaker.Margin = new Thickness(0, 0, 8, 0);
        row.Children.Add(speaker);
        row.Children.Add(new TextBlock
        {
            Text = "Audio devices",
            VerticalAlignment = VerticalAlignment.Center,
            FontSize = Size("SizeSmall"),
            Foreground = Res("TextDim"),
        });

        _button.Background = Res("Card");
        _button.BorderBrush = Res("Line");
        _button.BorderThickness = new Thickness(1);
        _button.CornerRadius = new CornerRadius(14);
        _button.Padding = new Thickness(14, 10, 14, 10);
        _button.Margin = new Thickness(0, 0, 7, 14);
        _button.VerticalAlignment = VerticalAlignment.Bottom;
        _button.Cursor = Cursors.Hand;
        _button.Child = row;
        _button.MouseLeftButtonUp += (_, _) => Toggle();
        _button.MouseEnter += (_, _) => _button.Background = Res("Hover");
        _button.MouseLeave += (_, _) => _button.Background = Res("Card");

        _overlay.PlacementTarget = _button;
        _overlay.Placement = PlacementMode.Bottom;
        _overlay.StaysOpen = false;
        _overlay.AllowsTransparency = true;
        _overlay.PopupAnimation = PopupAnimation.Fade;
        _overlay.VerticalOffset = 6;
        _overlay.Closed += (_, _) => _open = false;
    }

    private void Toggle()
    {
        _open = !_open;

        if (_mode == LayoutMode.Full)
        {
            Child = _open ? _expanded : _rail;
            return;
        }

        _overlay.IsOpen = _open;
    }

    /// <summary>Rail beside the strips, or a button in the band with an overlay.</summary>
    public void SetMode(LayoutMode mode)
    {
        if (mode == _mode)
        {
            return;
        }

        _mode = mode;
        _open = false;
        _overlay.IsOpen = false;

        if (_expanded.Parent is Decorator previous)
        {
            previous.Child = null;
        }

        if (mode == LayoutMode.Full)
        {
            _overlay.Child = null;
            Child = _rail;
        }
        else
        {
            _overlay.Child = new Border
            {
                Background = Res("PopupGround"),
                BorderBrush = Res("LineStrong"),
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(12),
                Padding = new Thickness(14),
                Child = _expanded,
                Effect = new System.Windows.Media.Effects.DropShadowEffect
                {
                    BlurRadius = 20,
                    ShadowDepth = 3,
                    Opacity = (double)Application.Current.Resources["ShadowOpacity"],
                    Color = Colors.Black,
                },
            };

            Child = _button;
        }
    }

    /// <summary>Redraw the list, but only when something it shows has changed.</summary>
    /// <remarks>
    /// State arrives about thirty times a second while meters run. The list is
    /// rebuilt when an entry's position, enabled flag, presence or name
    /// changes - which is exactly when a device is plugged in, and is what
    /// makes the list update live like the rest of the window.
    /// </remarks>
    public void Update(IReadOnlyList<PriorityState>? list)
    {
        if (list is null)
        {
            return;
        }

        var signature = string.Join(";", list.Select(e => $"{e.Id}:{e.Enabled}:{e.Present}:{e.Name}"));
        if (signature == _signature)
        {
            return;
        }

        _signature = signature;
        _list = list;
        _rows.Children.Clear();

        var enabled = list.Where(e => e.Enabled).ToList();
        var disabled = list.Where(e => !e.Enabled).ToList();

        if (list.Count == 0)
        {
            _rows.Children.Add(Note("No output devices yet."));
            return;
        }

        for (var i = 0; i < enabled.Count; i++)
        {
            _rows.Children.Add(EnabledRow(enabled[i], i, enabled.Count));
        }

        if (enabled.Count == 0)
        {
            // Stated rather than left blank: with everything disabled, an
            // unplugged device sends its channels to the system default, and
            // that should not come as a surprise.
            _rows.Children.Add(Note("Nothing enabled - channels fall back to the Windows default."));
        }

        if (disabled.Count > 0)
        {
            _rows.Children.Add(new TextBlock
            {
                Text = "Disabled",
                Foreground = Res("TextGhost"),
                FontSize = Size("SizeTiny"),
                Margin = new Thickness(4, 12, 0, 4),
            });

            foreach (var entry in disabled)
            {
                _rows.Children.Add(DisabledRow(entry));
            }
        }
    }

    private UIElement EnabledRow(PriorityState entry, int index, int count)
    {
        var row = new DockPanel { Height = 32, LastChildFill = true };

        var rank = new TextBlock
        {
            Text = (index + 1).ToString(),
            Width = 18,
            Foreground = Res("TextGhost"),
            FontWeight = FontWeights.SemiBold,
            VerticalAlignment = VerticalAlignment.Center,
            TextAlignment = TextAlignment.Right,
            Margin = new Thickness(0, 0, 10, 0),
        };
        DockPanel.SetDock(rank, Dock.Left);
        row.Children.Add(rank);

        var disable = TextButton("Disable", "Never use this device as a fallback");
        disable.MouseLeftButtonUp += (_, _) => Send(entry.Id, enabled: false, move: 0);
        DockPanel.SetDock(disable, Dock.Right);
        row.Children.Add(disable);

        var down = ArrowButton(Icons.Direction.Down, "Lower priority", index < count - 1);
        down.MouseLeftButtonUp += (_, _) => Send(entry.Id, enabled: true, move: +1);
        DockPanel.SetDock(down, Dock.Right);
        row.Children.Add(down);

        var up = ArrowButton(Icons.Direction.Up, "Higher priority", index > 0);
        up.MouseLeftButtonUp += (_, _) => Send(entry.Id, enabled: true, move: -1);
        DockPanel.SetDock(up, Dock.Right);
        row.Children.Add(up);

        row.Children.Add(DeviceLabel(entry, dim: false));
        return Framed(row);
    }

    private UIElement DisabledRow(PriorityState entry)
    {
        var row = new DockPanel { Height = 32, LastChildFill = true, Opacity = 0.55 };

        var spacer = new Border { Width = 28 };
        DockPanel.SetDock(spacer, Dock.Left);
        row.Children.Add(spacer);

        var enable = TextButton("Enable", "Use this device as a fallback again");
        enable.MouseLeftButtonUp += (_, _) => Send(entry.Id, enabled: true, move: 0);
        DockPanel.SetDock(enable, Dock.Right);
        row.Children.Add(enable);

        row.Children.Add(DeviceLabel(entry, dim: true));
        return Framed(row);
    }

    private static UIElement DeviceLabel(PriorityState entry, bool dim)
    {
        var panel = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        panel.Children.Add(new TextBlock
        {
            Text = Short(entry.Name),
            ToolTip = entry.Name,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = Res(dim || !entry.Present ? "TextFaint" : "Text"),
        });

        // An unplugged device keeps its place, and says so in words.
        if (!entry.Present)
        {
            panel.Children.Add(new TextBlock
            {
                Text = "unplugged",
                FontSize = Size("SizeTiny"),
                Foreground = Res("TextGhost"),
            });
        }

        return panel;
    }

    private static Border Framed(UIElement content) =>
        new()
        {
            Padding = new Thickness(4, 0, 4, 0),
            Margin = new Thickness(0, 0, 0, 2),
            CornerRadius = new CornerRadius(8),
            Background = Res("Raised"),
            Child = content,
        };

    /// <summary>
    /// Work out the whole new order and hand it to the core.
    /// </summary>
    /// <remarks>
    /// The core is sent the complete list rather than "move this one up", so
    /// what it stores is exactly what the user was looking at. It normalises
    /// on its side too - disabled entries always end up at the bottom - so a
    /// click here cannot produce an order the core would not accept.
    /// </remarks>
    private void Send(string id, bool enabled, int move)
    {
        var items = _list.Select(e => (e.Id, e.Enabled)).ToList();
        var at = items.FindIndex(e => e.Id == id);
        if (at < 0)
        {
            return;
        }

        items[at] = (id, enabled);

        if (move != 0)
        {
            var target = at + move;
            if (target < 0 || target >= items.Count || !items[target].Enabled)
            {
                return;
            }

            (items[at], items[target]) = (items[target], items[at]);
        }
        else if (!enabled)
        {
            // Disabling pins it below everything, which is where the core will
            // put it anyway; doing it here too keeps the two in agreement.
            var item = items[at];
            items.RemoveAt(at);
            items.Add(item);
        }
        else
        {
            // Enabling puts it at the bottom of the enabled ones.
            var item = items[at];
            items.RemoveAt(at);
            items.Insert(items.Count(e => e.Enabled), item);
        }

        PriorityChanged?.Invoke(items);
    }

    private static Border RoundButton(UIElement content, string tooltip)
    {
        var button = new Border
        {
            Width = 30,
            Height = 30,
            CornerRadius = new CornerRadius(10),
            Background = Res("Card"),
            BorderBrush = Res("Line"),
            BorderThickness = new Thickness(1),
            Cursor = Cursors.Hand,
            HorizontalAlignment = HorizontalAlignment.Center,
            ToolTip = tooltip,
            Child = content,
        };
        if (content is FrameworkElement fe)
        {
            fe.HorizontalAlignment = HorizontalAlignment.Center;
            fe.VerticalAlignment = VerticalAlignment.Center;
        }
        button.MouseEnter += (_, _) => button.Background = Res("Hover");
        button.MouseLeave += (_, _) => button.Background = Res("Card");
        return button;
    }

    private static Border ArrowButton(Icons.Direction direction, string tooltip, bool usable)
    {
        var chevron = Icons.Chevron(direction, Res(usable ? "TextDim" : "Line"), 10);
        chevron.HorizontalAlignment = HorizontalAlignment.Center;
        chevron.VerticalAlignment = VerticalAlignment.Center;

        var button = new Border
        {
            Width = 24,
            Height = 24,
            Margin = new Thickness(2, 0, 0, 0),
            CornerRadius = new CornerRadius(6),
            Background = Brushes.Transparent,
            VerticalAlignment = VerticalAlignment.Center,
            Cursor = usable ? Cursors.Hand : Cursors.Arrow,
            IsHitTestVisible = usable,
            ToolTip = tooltip,
            Child = chevron,
        };
        button.MouseEnter += (_, _) => button.Background = Res("LineStrong");
        button.MouseLeave += (_, _) => button.Background = Brushes.Transparent;
        return button;
    }

    private static Border TextButton(string text, string tooltip)
    {
        var button = new Border
        {
            Margin = new Thickness(6, 0, 0, 0),
            Padding = new Thickness(7, 3, 7, 3),
            CornerRadius = new CornerRadius(6),
            Background = Brushes.Transparent,
            VerticalAlignment = VerticalAlignment.Center,
            Cursor = Cursors.Hand,
            ToolTip = tooltip,
            Child = new TextBlock
            {
                Text = text,
                FontSize = Size("SizeTiny"),
                Foreground = Res("TextFaint"),
            },
        };
        button.MouseEnter += (_, _) => button.Background = Res("LineStrong");
        button.MouseLeave += (_, _) => button.Background = Brushes.Transparent;
        return button;
    }

    private static TextBlock Note(string text) =>
        new()
        {
            Text = text,
            Foreground = Res("TextGhost"),
            FontSize = Size("SizeTiny"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(4, 6, 4, 0),
        };

    /// <summary>"Speakers (Realtek(R) Audio)" reads better as "Realtek(R) Audio".</summary>
    private static string Short(string name)
    {
        var open = name.IndexOf('(');
        var close = name.LastIndexOf(')');
        return close > open + 1 && open >= 0 ? name[(open + 1)..close] : name;
    }
}
