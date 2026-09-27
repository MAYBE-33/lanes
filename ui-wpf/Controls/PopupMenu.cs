using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// A small right-click style menu in the window's own style.
/// </summary>
/// <remarks>
/// WPF's <c>ContextMenu</c> brings the system's look with it - a light box, a
/// gutter for icons nobody has, a blue highlight - and restyling it fully is a
/// template longer than this class. The app chips' menu was built by hand for
/// that reason; this is that menu made reusable, so the channel menu is the
/// same object rather than a near copy of it.
/// </remarks>
internal sealed class PopupMenu
{
    private readonly StackPanel _items = new();
    private readonly Popup _popup;

    public PopupMenu(UIElement target, double minWidth = 210)
    {
        _popup = new Popup
        {
            PlacementTarget = target,
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
                MinWidth = minWidth,
                Child = _items,
            },
        };
    }

    public void Open() => _popup.IsOpen = true;

    public void Close() => _popup.IsOpen = false;

    public void Add(UIElement element) => _items.Children.Add(element);

    /// <summary>A quiet label over a group of rows.</summary>
    public void Heading(string text) => _items.Children.Add(new TextBlock
    {
        Text = text,
        FontSize = (double)Application.Current.Resources["SizeTiny"],
        Foreground = Res("TextFaint"),
        Margin = new Thickness(8, 4, 8, 4),
    });

    public void Separator() =>
        _items.Children.Add(new Border { Height = 1, Background = Res("LineStrong"), Margin = new Thickness(6, 4, 6, 4) });

    /// <summary>
    /// A row that closes the menu and acts.
    /// </summary>
    /// <param name="note">A word on the right, such as "current", for the row
    /// that describes how things already are.</param>
    public Border Row(string text, Action act, string? tooltip = null, string? note = null)
    {
        var row = PlainRow(text, tooltip, note);
        row.MouseLeftButtonUp += (_, _) =>
        {
            Close();
            act();
        };
        _items.Children.Add(row);
        return row;
    }

    /// <summary>
    /// A row that needs a second click, for anything that cannot be undone
    /// from here. The first click arms it and says so; moving off disarms it.
    /// </summary>
    public void ConfirmRow(string text, string armed, Action act, string? tooltip = null)
    {
        var row = PlainRow(text, tooltip, null);
        var label = (TextBlock)((DockPanel)row.Child).Children[^1];
        var isArmed = false;

        row.MouseLeftButtonUp += (_, _) =>
        {
            if (!isArmed)
            {
                isArmed = true;
                label.Text = armed;
                label.Foreground = Res("Warn");
                return;
            }

            Close();
            act();
        };
        row.MouseLeave += (_, _) =>
        {
            isArmed = false;
            label.Text = text;
            label.Foreground = Res("TextDim");
        };
        _items.Children.Add(row);
    }

    private static Border PlainRow(string text, string? tooltip, string? note)
    {
        var content = new DockPanel { LastChildFill = true };
        if (note is not null)
        {
            var noted = new TextBlock
            {
                Text = note,
                Margin = new Thickness(12, 0, 8, 0),
                VerticalAlignment = VerticalAlignment.Center,
                FontSize = (double)Application.Current.Resources["SizeTiny"],
                Foreground = Res("TextGhost"),
            };
            DockPanel.SetDock(noted, Dock.Right);
            content.Children.Add(noted);
        }

        content.Children.Add(new TextBlock
        {
            Text = text,
            Margin = new Thickness(8, 0, 8, 0),
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = Res(note is null ? "TextDim" : "Text"),
        });

        var row = new Border
        {
            Height = 28,
            CornerRadius = new CornerRadius(6),
            Background = Brushes.Transparent,
            Cursor = Cursors.Hand,
            ToolTip = tooltip,
            Child = content,
        };
        row.MouseEnter += (_, _) => row.Background = Res("LineStrong");
        row.MouseLeave += (_, _) => row.Background = Brushes.Transparent;
        return row;
    }

    private static Brush Res(string key) => (Brush)Application.Current.Resources[key];
}
