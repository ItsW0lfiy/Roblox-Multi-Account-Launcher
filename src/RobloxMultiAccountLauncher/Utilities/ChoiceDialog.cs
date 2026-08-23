using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace RobloxMultiAccountLauncher.Utilities;

public static class ChoiceDialog
{
    public static int Show(Window owner, string title, string message, params string[] choices)
    {
        var dialog = new Window
        {
            Owner = owner,
            Title = title,
            Width = 520,
            SizeToContent = SizeToContent.Height,
            ResizeMode = ResizeMode.NoResize,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ShowInTaskbar = false,
            Background = (Brush)System.Windows.Application.Current.Resources["BackgroundBrush"],
            Foreground = (Brush)System.Windows.Application.Current.Resources["TextBrush"]
        };
        var panel = new StackPanel { Margin = new Thickness(22) };
        panel.Children.Add(new TextBlock { Text = title, FontSize = 18, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 0, 0, 12) });
        panel.Children.Add(new TextBlock { Text = message, TextWrapping = TextWrapping.Wrap, Foreground = (Brush)System.Windows.Application.Current.Resources["SecondaryTextBrush"], Margin = new Thickness(0, 0, 0, 22) });
        var buttons = new WrapPanel { HorizontalAlignment = HorizontalAlignment.Right };
        var result = -1;
        for (var index = 0; index < choices.Length; index++)
        {
            var captured = index;
            var button = new Button { Content = choices[index], MinWidth = 105, Margin = new Thickness(8, 0, 0, 0) };
            if (index == choices.Length - 1) button.Style = (Style)System.Windows.Application.Current.Resources["PrimaryButton"];
            button.Click += (_, _) => { result = captured; dialog.DialogResult = true; };
            buttons.Children.Add(button);
        }
        panel.Children.Add(buttons);
        dialog.Content = panel;
        dialog.ShowDialog();
        return result;
    }
}
