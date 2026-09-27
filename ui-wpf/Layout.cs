namespace Lanes.Ui;

/// <summary>How the mixer is arranged at the window's current size.</summary>
public enum LayoutMode
{
    /// <summary>Pool as a left column, strips across the rest. The desktop shape.</summary>
    Full,

    /// <summary>
    /// Strips still vertical but narrowed; the pool becomes a header button.
    /// </summary>
    Compact,

    /// <summary>
    /// Channels as full-width rows with horizontal faders. "Mobile mode".
    /// </summary>
    Stacked,
}

/// <summary>
/// Chooses the layout for a window size, and refuses to change its mind too
/// easily.
/// </summary>
/// <remarks>
/// <para>
/// <b>Hysteresis is the whole reason this is a type and not an if-statement.</b>
/// A single threshold means that resting the window edge exactly on it makes
/// the layout flip back and forth with every pixel of mouse jitter — and each
/// flip re-parents every control in the window. Transitions should be
/// immediate and stable, and those two words pull in opposite directions
/// unless the boundary is a band rather than a line.
/// </para>
/// <para>
/// So each boundary has two numbers: a width at which the layout gives way, and
/// a wider one at which it comes back. Inside the gap, <b>whatever is already
/// showing stays</b>. A slow drag therefore switches once and stays switched;
/// it cannot oscillate, because leaving a mode requires travelling further than
/// entering it did.
/// </para>
/// <para>
/// The numbers - roughly 900 for compact and 560 for stacked - are what the
/// content actually needs. The gap is 48
/// logical pixels, which is wider than any plausible hand tremor and narrow
/// enough that nobody notices the asymmetry.
/// </para>
/// </remarks>
public static class Layout
{
    /// <summary>Below this width, the pool folds away and the strips narrow.</summary>
    private const double CompactBelow = 900;

    /// <summary>Below this, channels become rows and the faders lie down.</summary>
    private const double StackedBelow = 560;

    /// <summary>How much wider it has to get before a mode is given back.</summary>
    private const double Gap = 48;

    /// <summary>
    /// A window taller than it is wide is stacked even when it is not narrow.
    /// </summary>
    /// <remarks>
    /// Stacked applies when the window is narrow, <i>or</i> portrait below a
    /// width threshold. A tall narrow window is a column of rows
    /// whatever its exact width: six vertical faders in a portrait window waste
    /// the one dimension there is plenty of.
    ///
    /// The width limit is what stops a large portrait display — a rotated
    /// 1440x2560, say — from being treated as a phone.
    /// </remarks>
    private const double PortraitBelow = 1000;

    /// <summary>
    /// The layout for this size, given what is on screen now.
    /// </summary>
    /// <param name="width">Window width in logical pixels.</param>
    /// <param name="height">Window height in logical pixels.</param>
    /// <param name="current">What is showing. This is what makes it sticky.</param>
    public static LayoutMode For(double width, double height, LayoutMode current)
    {
        // Portrait first: it can force Stacked at a width that would otherwise
        // be Compact or even Full, so testing it after the width bands would
        // mean the width answer sometimes silently wins.
        var portrait = height > width && width < PortraitBelow;

        // Each threshold is read in the direction the window is actually
        // moving. `Leaving` is the wider number, used only when the mode is
        // already active - that asymmetry IS the hysteresis.
        var stacked = portrait || Narrower(width, StackedBelow, current == LayoutMode.Stacked);
        if (stacked)
        {
            return LayoutMode.Stacked;
        }

        return Narrower(width, CompactBelow, current == LayoutMode.Compact)
            ? LayoutMode.Compact
            : LayoutMode.Full;
    }

    /// <summary>
    /// Is the window narrow enough for this mode, allowing for stickiness?
    /// </summary>
    /// <param name="active">
    /// Whether this mode is the one currently showing. When it is, the window
    /// has to grow past <paramref name="threshold"/> + <see cref="Gap"/> to
    /// leave; when it is not, it only has to shrink past the threshold to
    /// enter.
    /// </param>
    private static bool Narrower(double width, double threshold, bool active) =>
        width < (active ? threshold + Gap : threshold);
}
