using System.Windows;
using System.Windows.Media;

namespace Lanes.Ui.Controls;

/// <summary>
/// The detent scale, its magnetism, and the shape it is drawn as.
/// </summary>
/// <remarks>
/// <para>
/// Shared by the channel faders and the Game/Chat mix so the two behave
/// <i>identically</i>: the same method of control and the same magnetism. Two
/// copies of these numbers would agree today and drift the first time one was
/// tuned.
/// </para>
/// <para>
/// Everything here is expressed as a <b>fraction of travel</b> rather than as a
/// value, so it applies unchanged to a fader running 0 to 1 and to a balance
/// running -1 to +1. The mix's centre detent is fraction 0.5 on both.
/// </para>
/// </remarks>
internal static class Detents
{
    /// <summary>Where the detents sit, as a fraction of the control's travel.</summary>
    public static readonly double[] Positions = [0.0, 0.25, 0.5, 0.75, 1.0];

    /// <summary>
    /// How close a release has to be, as a fraction of travel, to be pulled in.
    /// </summary>
    /// <remarks>
    /// The user's words about the faders: the slider should naturally want to
    /// fall to 50% if the drag is released at 51%, but not be so magnetic that
    /// 55% cannot be set. Four percent does that - 51 and 53 snap, 55 does not
    /// - and anything much larger starts taking the control away.
    ///
    /// It is only half the rule. See <see cref="DwellToCommit"/>.
    /// </remarks>
    public const double SnapRadius = 0.04;

    /// <summary>
    /// How long to hold still before a value counts as deliberate.
    /// </summary>
    /// <remarks>
    /// <b>The magnetism gets out of the way when it is clear you meant it.</b>
    /// A detent is a convenience for a quick gesture - flick roughly halfway
    /// and it tidies to exactly half. Someone who slides to 52% and
    /// <i>pauses</i> is not aiming at 50%; they have found the value they want.
    /// Snapping then is the control overruling a decision it just watched being
    /// made.
    ///
    /// Long enough not to fire during an ordinary sweep, short enough that
    /// deliberately stopping feels like it counts.
    ///
    /// <b>Read at release, never latched during the drag.</b> A pointer held
    /// genuinely still produces no move events at all, so a check in the move
    /// handler never runs - it works for a hand with a tremor and fails for a
    /// steady one, which is the worst kind of intermittent.
    /// </remarks>
    public static readonly TimeSpan DwellToCommit = TimeSpan.FromMilliseconds(350);

    // --- The paddle --------------------------------------------------------
    //
    // Sketched by the user: a wide head on the outside that tapers into a thin
    // stem, and the stem runs THROUGH the track rather than stopping at it.

    /// <summary>The wide part, on the outside.</summary>
    public const double HeadWidth = 13;

    public const double HeadHeight = 9;

    /// <summary>The diagonal shoulder between the head and the stem.</summary>
    public const double TaperWidth = 6;

    /// <summary>The thin part, which crosses the track and comes out the far side.</summary>
    public const double StemHeight = 3;

    public const double StemOvershoot = 4;

    /// <summary>The gap between the control's outer edge and the head.</summary>
    public const double EdgeInset = 2;

    /// <summary>How far beyond the drawn head still counts as a click on it.</summary>
    /// <remarks>
    /// Small, deliberately. It was once five pixels <i>and</i> the whole column
    /// beside the track belonged to the nearest detent, so the empty space
    /// between them was live and a click anywhere down the side jumped the
    /// value to a quarter. A target you did not aim at is not a convenience.
    /// </remarks>
    public const double HitPadding = 3;

    /// <summary>Pull to a detent if a release landed close enough to one.</summary>
    /// <param name="fraction">Position along the travel, 0 to 1.</param>
    public static double Snap(double fraction)
    {
        foreach (var detent in Positions)
        {
            if (Math.Abs(fraction - detent) <= SnapRadius)
            {
                return detent;
            }
        }

        return fraction;
    }

    /// <summary>
    /// One paddle, in local coordinates about the point it marks.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The origin is on the track's centre line at the detent's position.
    /// Negative <c>x</c> runs outward to the head; positive <c>x</c> runs in
    /// through the track and out the far side. A vertical fader draws this as
    /// it is; the horizontal mix rotates it a quarter turn, which is the whole
    /// reason it is built about the origin rather than in place.
    /// </para>
    /// <para>
    /// <b>One closed figure, not three overlapping shapes.</b> The outline is
    /// continuous and the corners round as one; drawn in pieces it leaves seams
    /// wherever a fill meets a stroke.
    /// </para>
    /// </remarks>
    /// <param name="outer">Distance from the track's centre to the head's outer edge.</param>
    /// <param name="trackHalf">Half the track's thickness.</param>
    /// <param name="transform">Placement, and rotation for a horizontal control.</param>
    public static Geometry Paddle(double outer, double trackHalf, Transform transform)
    {
        var headOuter = -outer;
        var headInner = headOuter + HeadWidth;
        var stemStart = headInner + TaperWidth;
        var stemEnd = trackHalf + StemOvershoot;

        var headTop = -HeadHeight / 2;
        var headBottom = HeadHeight / 2;
        var stemTop = -StemHeight / 2;
        var stemBottom = StemHeight / 2;
        const double corner = 3.0;

        var geometry = new StreamGeometry();
        using (var context = geometry.Open())
        {
            // Clockwise from the head's outer top corner, rounding the outer
            // corners at each end and leaving the shoulders as clean diagonals.
            context.BeginFigure(new Point(headOuter + corner, headTop), true, true);

            context.LineTo(new Point(headInner, headTop), true, true);
            context.LineTo(new Point(stemStart, stemTop), true, true);
            context.LineTo(new Point(stemEnd - corner, stemTop), true, true);
            context.ArcTo(
                new Point(stemEnd - corner, stemBottom), new Size(corner, corner),
                0, false, SweepDirection.Clockwise, true, true);
            context.LineTo(new Point(stemStart, stemBottom), true, true);
            context.LineTo(new Point(headInner, headBottom), true, true);
            context.LineTo(new Point(headOuter + corner, headBottom), true, true);
            context.ArcTo(
                new Point(headOuter, headBottom - corner), new Size(corner, corner),
                0, false, SweepDirection.Clockwise, true, true);
            context.LineTo(new Point(headOuter, headTop + corner), true, true);
            context.ArcTo(
                new Point(headOuter + corner, headTop), new Size(corner, corner),
                0, false, SweepDirection.Clockwise, true, true);
        }

        geometry.Transform = transform;
        geometry.Freeze();
        return geometry;
    }

    /// <summary>The head's hit box in local coordinates.</summary>
    /// <remarks>
    /// Head and shoulder only. The stem crosses the track, and a click there
    /// belongs to the track.
    /// </remarks>
    public static Rect HeadHitBox(double outer) =>
        new(
            -outer - HitPadding,
            -HeadHeight / 2 - HitPadding,
            HeadWidth + TaperWidth + HitPadding,
            HeadHeight + 2 * HitPadding);
}
