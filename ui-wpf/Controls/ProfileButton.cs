using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

using Lanes.Ui.Api;

namespace Lanes.Ui.Controls;

/// <summary>
/// The profile picker: shows which profile the mixer is in, and opens a list to
/// switch, save and delete.
/// </summary>
/// <remarks>
/// <para>
/// Shaped like <see cref="DeviceButton"/> - a button that opens a list - so the
/// window has one idiom for "pick one of these" rather than two.
/// </para>
/// <para>
/// <b>What the label says is what the core says.</b> The core reports a profile
/// as active only while the mixer actually matches it (see <c>profiles::active</c>
/// in the core), so moving any fader turns "Gaming" into "Custom" here without
/// this control having to know why. Nothing in the window keeps its own idea of
/// which profile is on.
/// </para>
/// <para>
/// <b>Deleting takes two clicks.</b> A profile is a saved arrangement of six
/// channels that took some care to get right, and the delete mark sits a few
/// pixels from the row that switches to it. The first click arms it; the second
/// deletes; moving off the row disarms it.
/// </para>
/// </remarks>
public sealed class ProfileButton : Border
{
    private readonly TextBlock _caption = new()
    {
        Text = "Profile",
        VerticalAlignment = VerticalAlignment.Center,
        Margin = new Thickness(0, 0, 8, 0),
        Visibility = Visibility.Collapsed,
    };

    private readonly TextBlock _label = new()
    {
        VerticalAlignment = VerticalAlignment.Center,
        TextTrimming = TextTrimming.CharacterEllipsis,
    };

    private readonly Popup _popup;
    private readonly StackPanel _items = new();
    private ProfileState _state = new([], null);
    private string _signature = "\0";

    /// <summary>Switch to the named profile.</summary>
    public event Action<string>? ActivateRequested;

    /// <summary>Save the mixer as it is now under this name.</summary>
    public event Action<string>? SaveRequested;

    /// <summary>Delete the named profile.</summary>
    public event Action<string>? DeleteRequested;

    /// <summary>
    /// Say "Profile" inside the button, for layouts with no room for a label
    /// above it.
    /// </summary>
    public bool ShowCaption
    {
        get => _caption.Visibility == Visibility.Visible;
        set => _caption.Visibility = value ? Visibility.Visible : Visibility.Collapsed;
    }

    public ProfileButton()
    {
        Background = Res("Raised");
        CornerRadius = new CornerRadius(9);
        MinWidth = 34;
        Height = 34;
        Cursor = Cursors.Hand;
        SnapsToDevicePixels = true;
        ToolTip = "Switch, save or delete profiles";

        _caption.FontSize = (double)Application.Current.Resources["SizeTiny"];
        _caption.Foreground = Res("TextFaint");

        var layout = new DockPanel { LastChildFill = true, Margin = new Thickness(11, 0, 10, 0) };

        var chevron = Icons.Chevron(Icons.Direction.Down, Res("TextGhost"), 9);
        chevron.VerticalAlignment = VerticalAlignment.Center;
        chevron.Margin = new Thickness(7, 0, 0, 0);
        DockPanel.SetDock(chevron, Dock.Right);
        DockPanel.SetDock(_caption, Dock.Left);

        layout.Children.Add(chevron);
        layout.Children.Add(_caption);
        layout.Children.Add(_label);
        Child = layout;

        MouseEnter += (_, _) => Opacity = 0.8;
        MouseLeave += (_, _) => Opacity = 1.0;

        _popup = new Popup
        {
            PlacementTarget = this,
            // The button lives in the band across the top of the window, so
            // the list opens downwards, over the strips, rather than off the
            // top of the screen.
            Placement = PlacementMode.Bottom,
            VerticalOffset = 6,
            StaysOpen = false,
            AllowsTransparency = true,
            PopupAnimation = PopupAnimation.Fade,
        };

        _popup.Child = new Border
        {
            Background = Res("PopupGround"),
            BorderBrush = Res("LineStrong"),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(10),
            Padding = new Thickness(5),
            MinWidth = 220,
            MaxWidth = 300,
            Effect = new System.Windows.Media.Effects.DropShadowEffect
            {
                BlurRadius = 18,
                ShadowDepth = 2,
                Opacity = (double)Application.Current.Resources["ShadowOpacity"],
                Color = Colors.Black,
            },
            Child = _items,
        };

        // Rebuilt on every opening, so a half-typed name or an armed delete
        // from last time is never waiting when it is opened again.
        MouseLeftButtonUp += (_, _) =>
        {
            Rebuild();
            _popup.IsOpen = true;
        };

        Update(_state);
    }

    /// <summary>Refresh from the core's state.</summary>
    public void Update(ProfileState state)
    {
        _state = state;

        _label.Text = state.Active ?? (state.Names.Count > 0 ? "Custom" : "None saved");
        _label.Foreground = Res(state.Active is null ? "TextGhost" : "TextDim");

        // Rebuilding under the pointer while the list is open would throw away
        // a name being typed, so an open list is only rebuilt when what it
        // shows has actually changed - a profile switched from the tray, say.
        var signature = string.Join("\n", state.Names) + "\0" + state.Active;
        if (signature != _signature)
        {
            _signature = signature;
            if (_popup.IsOpen && !_saving)
            {
                Rebuild();
            }
        }
    }

    /// <summary>Whether the "save as" box is open, which a rebuild would discard.</summary>
    private bool _saving;

    private void Rebuild()
    {
        _saving = false;
        _items.Children.Clear();

        if (_state.Names.Count == 0)
        {
            _items.Children.Add(new TextBlock
            {
                Text = "No profiles yet. Set the mixer up the way you want it, then save it here.",
                TextWrapping = TextWrapping.Wrap,
                FontSize = (double)Application.Current.Resources["SizeSmall"],
                Foreground = Res("TextFaint"),
                Margin = new Thickness(10, 8, 10, 8),
            });
        }

        foreach (var name in _state.Names)
        {
            _items.Children.Add(ProfileRow(name, string.Equals(name, _state.Active, StringComparison.OrdinalIgnoreCase)));
        }

        _items.Children.Add(new Border
        {
            Height = 1,
            Background = Res("LineStrong"),
            Margin = new Thickness(6, 5, 6, 5),
        });

        _items.Children.Add(SaveRow());
    }

    private Border ProfileRow(string name, bool active)
    {
        var text = new TextBlock
        {
            Text = name,
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontWeight = active ? FontWeights.Bold : FontWeights.Normal,
            Foreground = Res(active ? "Text" : "TextDim"),
        };

        // Where "active" is said in words as well as weight, because bold on
        // its own is easy to miss in a short list.
        var state = new TextBlock
        {
            Text = active ? "on" : "",
            VerticalAlignment = VerticalAlignment.Center,
            FontSize = (double)Application.Current.Resources["SizeTiny"],
            Foreground = Res("TextGhost"),
            Margin = new Thickness(8, 0, 0, 0),
        };

        var delete = new Border
        {
            Padding = new Thickness(7, 3, 7, 3),
            CornerRadius = new CornerRadius(6),
            Background = Brushes.Transparent,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(6, 0, 0, 0),
            Visibility = Visibility.Hidden,
            ToolTip = "Delete this profile",
            Child = new TextBlock
            {
                Text = "Delete",
                FontSize = (double)Application.Current.Resources["SizeTiny"],
                Foreground = Res("TextFaint"),
            },
        };

        var armed = false;
        var deleteText = (TextBlock)delete.Child;

        void Disarm()
        {
            armed = false;
            deleteText.Text = "Delete";
            deleteText.Foreground = Res("TextFaint");
            delete.Background = Brushes.Transparent;
        }

        delete.MouseEnter += (_, _) => delete.Background = Res("Hover");
        delete.MouseLeave += (_, _) =>
        {
            if (!armed)
            {
                delete.Background = Brushes.Transparent;
            }
        };
        delete.MouseLeftButtonUp += (_, e) =>
        {
            // The row underneath switches profile on the same click otherwise.
            e.Handled = true;

            if (!armed)
            {
                armed = true;
                deleteText.Text = "Delete?";
                deleteText.Foreground = Res("Warn");
                delete.Background = Res("WarnGround");
                return;
            }

            _popup.IsOpen = false;
            DeleteRequested?.Invoke(name);
        };

        var content = new DockPanel { Margin = new Thickness(10, 0, 4, 0) };
        DockPanel.SetDock(delete, Dock.Right);
        DockPanel.SetDock(state, Dock.Right);
        content.Children.Add(delete);
        content.Children.Add(state);
        content.Children.Add(text);

        var row = Row(content, active ? $"{name} - the mixer matches this profile" : $"Switch to {name}");

        // The delete mark appears only on the row being pointed at. Six
        // "Delete" words down the side of a short list would make deleting look
        // like what the list is for.
        row.MouseEnter += (_, _) => delete.Visibility = Visibility.Visible;
        row.MouseLeave += (_, _) =>
        {
            delete.Visibility = Visibility.Hidden;
            Disarm();
        };

        row.MouseLeftButtonUp += (_, _) =>
        {
            _popup.IsOpen = false;
            ActivateRequested?.Invoke(name);
        };

        return row;
    }

    /// <summary>"Save current as…", which opens into a name box in place.</summary>
    private Border SaveRow()
    {
        var text = new TextBlock
        {
            Text = "Save current as…",
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = Res("TextDim"),
            Margin = new Thickness(10, 0, 10, 0),
        };

        var row = Row(text, "Save every channel's volume, mute and device, and the mix, as a profile");
        row.MouseLeftButtonUp += (_, _) =>
        {
            var index = _items.Children.IndexOf(row);
            _items.Children.RemoveAt(index);
            _items.Children.Insert(index, NameBox());
        };
        return row;
    }

    private StackPanel NameBox()
    {
        _saving = true;

        var box = new TextBox
        {
            MaxLength = 40,
            Height = 30,
            Padding = new Thickness(6, 0, 6, 0),
            VerticalContentAlignment = VerticalAlignment.Center,
            Background = Res("Raised"),
            Foreground = Res("Text"),
            CaretBrush = Res("Text"),
            BorderBrush = Res("LineStrong"),
            BorderThickness = new Thickness(1),
            FontFamily = (FontFamily)Application.Current.Resources["UiFont"],
            // Handing it the active profile's name would make "save" an
            // overwrite by default. An empty box makes it a new profile unless
            // the user types an existing name - which the hint then calls out.
            Text = "",
        };

        var hint = new TextBlock
        {
            Text = "Enter to save · Esc to cancel",
            FontSize = (double)Application.Current.Resources["SizeTiny"],
            Foreground = Res("TextGhost"),
            Margin = new Thickness(2, 5, 2, 2),
            TextWrapping = TextWrapping.Wrap,
        };

        box.TextChanged += (_, _) =>
        {
            var name = box.Text.Trim();
            var existing = _state.Names.FirstOrDefault(n => string.Equals(n, name, StringComparison.OrdinalIgnoreCase));

            // Saving over a name replaces that profile. Said before it happens,
            // not after.
            if (existing is not null)
            {
                hint.Text = $"Replaces \"{existing}\" · Enter to save";
                hint.Foreground = Res("Warn");
            }
            else
            {
                hint.Text = "Enter to save · Esc to cancel";
                hint.Foreground = Res("TextGhost");
            }
        };

        box.KeyDown += (_, e) =>
        {
            if (e.Key == Key.Enter)
            {
                var name = box.Text.Trim();
                if (name.Length > 0)
                {
                    _popup.IsOpen = false;
                    SaveRequested?.Invoke(name);
                }
                e.Handled = true;
            }
            else if (e.Key == Key.Escape)
            {
                Rebuild();
                e.Handled = true;
            }
        };

        var panel = new StackPanel { Margin = new Thickness(5, 3, 5, 3) };
        panel.Children.Add(box);
        panel.Children.Add(hint);

        // After layout, or the box is not yet focusable and the first key
        // pressed goes to the window underneath.
        Dispatcher.BeginInvoke(() =>
        {
            box.Focus();
            Keyboard.Focus(box);
        }, System.Windows.Threading.DispatcherPriority.Input);

        return panel;
    }

    /// <summary>The hover and hit behaviour every row shares. Same as the device list's.</summary>
    private static Border Row(UIElement content, string tooltip)
    {
        var row = new Border
        {
            Height = 30,
            CornerRadius = new CornerRadius(7),
            Background = Brushes.Transparent,
            Cursor = Cursors.Hand,
            Child = content,
            ToolTip = tooltip,
        };

        var hover = Res("LineStrong");
        row.MouseEnter += (_, _) => row.Background = hover;
        row.MouseLeave += (_, _) => row.Background = Brushes.Transparent;
        return row;
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
