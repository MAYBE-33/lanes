using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// Shrinks its content, uniformly, until it fits the window - height and width.
/// </summary>
/// <remarks>
/// <para>
/// <b>Why this exists.</b> The stacked layout is six full-width rows, and six
/// rows of a fixed height do not fit every portrait window. The obvious answer
/// is a scrollbar, and it is the wrong one. A mixer is a dashboard - the reason
/// to open it is to see every channel at once - and a channel you have to
/// scroll to is a channel you have stopped watching. So the window, the cards
/// and every element in them scale instead.
/// </para>
/// <para>
/// <b>Uniform, and by render transform rather than by layout.</b> Everything
/// shrinks together - type, faders, buttons, gaps - so the window stays the
/// design it was drawn as rather than becoming a different, denser one at every
/// size. A <c>LayoutTransform</c> would re-measure the content inside the scale
/// and let each control decide separately what to give up; a
/// <c>RenderTransform</c> scales the finished picture, which is the whole
/// point.
/// </para>
/// <para>
/// <b>The content is measured at the width it will have AFTER scaling</b>, not
/// at the window's width. Scaling down by 0.8 means the content is laid out in
/// a space 25% wider than the window in logical units and then drawn smaller,
/// so the fader is longer and the device names are less truncated - not more.
/// </para>
/// <para>
/// <b>Width too.</b> Six strips cannot go narrower than 110 pixels each, so a
/// 650-pixel window in the compact layout would otherwise cut the last strip in
/// half at the right edge.
/// </para>
/// <para>
/// Width cannot be detected the way height is. WPF clamps an element's
/// <c>DesiredSize</c> to the space it was offered, so content that needs more
/// width than it was given reports exactly the width it was given - the
/// overflow is invisible from here. The caller therefore states the minimum
/// through <see cref="MinimumContentWidth"/>, which it can work out exactly
/// because it knows how many strips it laid out.
/// </para>
/// <para>
/// <b>There is a floor, and below it the scroller comes back.</b> Scaling is a
/// way to fit a window that is a bit too short, not a way to render a mixer at
/// the size of a postage stamp. Past <see cref="MinimumScale"/> the content is
/// arranged taller than the slot, which is precisely what makes the
/// <c>ScrollViewer</c> inside it start scrolling again - an honest limit rather
/// than an illegible window.
/// </para>
/// </remarks>
public sealed class FitWindow : Decorator
{
    private readonly ScaleTransform _scale = new(1, 1);

    /// <summary>How far down it is willing to go before it gives up and scrolls.</summary>
    /// <remarks>
    /// 0.7 is about where the smallest type in the window - the app counts and
    /// the mix readout - stops being comfortably readable. There is no minimum
    /// window size beyond physical legibility, and this is where that limit
    /// actually is.
    /// </remarks>
    public double MinimumScale { get; set; } = 0.7;

    private double _minimumContentWidth;

    /// <summary>
    /// The narrowest the content can be laid out without clipping, in its own
    /// unscaled units. Zero means "no minimum".
    /// </summary>
    /// <remarks>See the class remarks for why this has to be told rather than measured.</remarks>
    public double MinimumContentWidth
    {
        get => _minimumContentWidth;
        set
        {
            if (Math.Abs(_minimumContentWidth - value) < 0.5)
            {
                return;
            }

            _minimumContentWidth = value;
            InvalidateMeasure();
        }
    }

    /// <summary>What the content is currently drawn at. 1 when it fits.</summary>
    public double Scale => _scale.ScaleX;

    public FitWindow()
    {
        // Past the floor the child is deliberately taller than this element.
        ClipToBounds = true;
    }

    protected override Size MeasureOverride(Size constraint)
    {
        var child = Child;
        if (child is null)
        {
            return new Size(0, 0);
        }

        child.RenderTransform = _scale;
        child.RenderTransformOrigin = new Point(0, 0);

        var width = constraint.Width;
        var height = constraint.Height;

        // Unbounded in either direction means nothing is asking it to fit, so
        // there is nothing to fit to. Measuring against infinity and then
        // dividing by a scale would be arithmetic on a number that has no
        // meaning.
        if (double.IsInfinity(width) || double.IsInfinity(height))
        {
            SetScale(1);
            child.Measure(constraint);
            return child.DesiredSize;
        }

        // Width first: if the content cannot be laid out as narrow as the
        // window, it is drawn smaller in a wider logical space - the same
        // trick as for height, and the same floor.
        var widthScale = _minimumContentWidth > width && _minimumContentWidth > 0
            ? Math.Max(MinimumScale, width / _minimumContentWidth)
            : 1.0;

        // What the content wants at the width it will have.
        child.Measure(new Size(width / widthScale, double.PositiveInfinity));
        var scale = Math.Min(widthScale, Fit(child.DesiredSize.Height, height));

        if (scale < widthScale)
        {
            // Ask again with the width the content will really have. A wider
            // row can be a shorter one - a device name that no longer needs to
            // wrap, a chip list that fits on one line - so the first answer is
            // an over-estimate and this pass gives some of the scale back.
            //
            // Once, not until it settles: the two feed into each other only
            // weakly, and a loop here runs inside layout.
            child.Measure(new Size(width / scale, double.PositiveInfinity));
            scale = Math.Min(widthScale, Fit(child.DesiredSize.Height, height));
        }

        SetScale(scale);
        child.Measure(new Size(width / scale, height / scale));

        return new Size(width, Math.Min(height, child.DesiredSize.Height * scale));
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var child = Child;
        if (child is null)
        {
            return finalSize;
        }

        // In the child's own units, which the transform then brings back down
        // to finalSize. Capped at the slot rather than at the content's natural
        // height, so that once the floor is reached the ScrollViewer inside is
        // genuinely short of room and shows its bar.
        child.Arrange(new Rect(0, 0, finalSize.Width / _scale.ScaleX, finalSize.Height / _scale.ScaleY));
        return finalSize;
    }

    private double Fit(double natural, double available) =>
        natural <= available || natural <= 0
            ? 1.0
            : Math.Max(MinimumScale, available / natural);

    private void SetScale(double scale)
    {
        // A render transform does not invalidate layout, so this is safe to
        // write from inside a measure pass - but only worth writing when it has
        // actually moved.
        if (Math.Abs(_scale.ScaleX - scale) > 0.0005)
        {
            _scale.ScaleX = scale;
            _scale.ScaleY = scale;
        }
    }
}
