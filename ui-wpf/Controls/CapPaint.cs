using System.Windows;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// The brushes a slider handle is drawn with, shared by the faders and the
/// Game/Chat mix.
/// </summary>
/// <remarks>
/// <para>
/// Shared because the two controls are the same object seen twice and ought to
/// look it. They did not: the faders were rebuilt with a dark grip and the mix
/// kept a plain white circle, which left the brightest thing in the window on
/// the one control that is centred and doing nothing most of the time.
/// </para>
/// <para>
/// <b>Dark, with the channel's colour across it.</b> A pale handle competes
/// with the text for attention and, on a fader, sits permanently against the
/// detent it is parked on — two bright shapes touching, reading as one. A dark
/// grip with a coloured index line reads as a handle resting on a scale, and
/// the line is where the value actually is.
/// </para>
/// <para>
/// Built once and frozen. These are drawn inside <c>OnRender</c>, which runs
/// every frame a meter is moving.
/// </para>
/// </remarks>
internal static class CapPaint
{
    private static Brush? _face;
    private static Brush? _mutedFace;
    private static Pen? _edge;

    /// <summary>The handle's fill, lit from above.</summary>
    public static Brush Face => _face ??= Build("CapTopColor", "CapBottomColor");

    /// <summary>The same, for a muted channel: flatter and closer to the track.</summary>
    public static Brush MutedFace => _mutedFace ??= Build("CapTopMutedColor", "CapBottomMutedColor");

    /// <summary>A hairline round the handle, so it holds its shape on the track.</summary>
    public static Pen Edge
    {
        get
        {
            if (_edge is null)
            {
                _edge = new Pen((Brush)Application.Current.Resources["CapEdge"], 1);
                _edge.Freeze();
            }

            return _edge;
        }
    }

    /// <summary>
    /// The index line across the handle: a 2px stroke in the given colour.
    /// </summary>
    public static Pen Index(Brush colour)
    {
        var pen = new Pen(colour, 2)
        {
            StartLineCap = PenLineCap.Round,
            EndLineCap = PenLineCap.Round,
        };
        pen.Freeze();
        return pen;
    }

    /// <summary>
    /// Forget the cached paints. Called when the palette changes: they are
    /// frozen, shared by every fader, and would otherwise keep the old theme's
    /// colours for the life of the process.
    /// </summary>
    public static void Reset()
    {
        _face = null;
        _mutedFace = null;
        _edge = null;
    }

    /// <summary>
    /// A vertical gradient between two colours from the theme.
    /// </summary>
    /// <remarks>
    /// The theme stores these as <c>Color</c> rather than <c>SolidColorBrush</c>
    /// precisely so they can be used as gradient stops, which a brush cannot be.
    /// </remarks>
    private static Brush Build(string topKey, string bottomKey)
    {
        var brush = new LinearGradientBrush
        {
            StartPoint = new Point(0, 0),
            EndPoint = new Point(0, 1),
        };
        brush.GradientStops.Add(new GradientStop((Color)Application.Current.Resources[topKey], 0));
        brush.GradientStops.Add(new GradientStop((Color)Application.Current.Resources[bottomKey], 1));
        brush.Freeze();
        return brush;
    }
}
