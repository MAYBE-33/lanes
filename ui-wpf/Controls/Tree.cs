using System.Windows;
using System.Windows.Controls;

namespace Lanes.Ui.Controls;

/// <summary>Moving long-lived controls between containers, safely.</summary>
/// <remarks>
/// <para>
/// The responsive layouts re-parent controls rather than rebuilding them, so
/// that a channel keeps its meter, its settled fader value and its open apps
/// list across a reflow. WPF makes that a little sharper than it looks: an
/// element may have exactly one logical parent, and adding a second throws
/// rather than moving it.
/// </para>
/// <para>
/// There is no single call for "take this out of whatever holds it", because a
/// parent may hold a child through a collection, a single <c>Child</c>, or a
/// <c>Content</c>. This covers those three and does nothing for an element that
/// has no parent yet — which is the common case on the first arrangement.
/// </para>
/// <para>
/// <b>Getting this wrong is not a cosmetic bug.</b> The exception is raised
/// from inside a <c>SizeChanged</c> handler, so the window does not misbehave,
/// it exits — while the user is dragging its edge.
/// </para>
/// </remarks>
internal static class Tree
{
    public static void Detach(FrameworkElement element)
    {
        switch (element.Parent)
        {
            case Panel panel:
                panel.Children.Remove(element);
                break;
            case Decorator decorator:
                decorator.Child = null;
                break;
            case ContentControl content:
                content.Content = null;
                break;
            case ContentPresenter presenter:
                presenter.Content = null;
                break;
        }
    }
}
