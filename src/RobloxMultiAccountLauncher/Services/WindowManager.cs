using System.Runtime.InteropServices;
using RobloxMultiAccountLauncher.Models;
using DrawingRectangle = System.Drawing.Rectangle;
using Screen = System.Windows.Forms.Screen;

namespace RobloxMultiAccountLauncher.Services;

public static class WindowManager
{
    private const uint SwpNoZOrder = 0x0004;
    private const int SwRestore = 9;

    public static IReadOnlyList<Screen> GetMonitors() => Screen.AllScreens;

    public static (DrawingRectangle First, DrawingRectangle Second) CalculateLayout(DrawingRectangle area, WindowLayoutMode mode, bool swapped = false)
    {
        DrawingRectangle first;
        DrawingRectangle second;
        switch (mode)
        {
            case WindowLayoutMode.PrimarySecondary:
                var primaryWidth = (int)Math.Round(area.Width * 0.70);
                first = new DrawingRectangle(area.Left, area.Top, primaryWidth, area.Height);
                second = new DrawingRectangle(area.Left + primaryWidth, area.Top, area.Width - primaryWidth, area.Height);
                break;
            case WindowLayoutMode.Vertical:
                var topHeight = area.Height / 2;
                first = new DrawingRectangle(area.Left, area.Top, area.Width, topHeight);
                second = new DrawingRectangle(area.Left, area.Top + topHeight, area.Width, area.Height - topHeight);
                break;
            default:
                var halfWidth = area.Width / 2;
                first = new DrawingRectangle(area.Left, area.Top, halfWidth, area.Height);
                second = new DrawingRectangle(area.Left + halfWidth, area.Top, area.Width - halfWidth, area.Height);
                break;
        }
        return swapped ? (second, first) : (first, second);
    }

    public static bool Tile(IReadOnlyList<RobloxClientInfo> clients, WindowLayoutMode mode, string? monitorDeviceName, bool swapped)
    {
        if (clients.Count != 2 || clients.Any(client => client.MainWindowHandle == nint.Zero)) return false;
        var screen = Screen.AllScreens.FirstOrDefault(s => s.DeviceName.Equals(monitorDeviceName, StringComparison.OrdinalIgnoreCase))
            ?? Screen.PrimaryScreen ?? Screen.AllScreens[0];
        var layout = CalculateLayout(screen.WorkingArea, mode, swapped);
        return Move(clients[0].MainWindowHandle, layout.First) & Move(clients[1].MainWindowHandle, layout.Second);
    }

    public static bool MoveToMonitor(RobloxClientInfo client, string? monitorDeviceName)
    {
        if (client.MainWindowHandle == nint.Zero) return false;
        var screen = Screen.AllScreens.FirstOrDefault(s => s.DeviceName.Equals(monitorDeviceName, StringComparison.OrdinalIgnoreCase))
            ?? Screen.PrimaryScreen ?? Screen.AllScreens[0];
        return Move(client.MainWindowHandle, screen.WorkingArea);
    }

    public static bool Focus(RobloxClientInfo client)
    {
        if (client.MainWindowHandle == nint.Zero) return false;
        ShowWindow(client.MainWindowHandle, SwRestore);
        return SetForegroundWindow(client.MainWindowHandle);
    }

    private static bool Move(nint handle, DrawingRectangle rectangle)
    {
        ShowWindow(handle, SwRestore);
        return SetWindowPos(handle, nint.Zero, rectangle.X, rectangle.Y, rectangle.Width, rectangle.Height, SwpNoZOrder);
    }

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(nint hWnd);

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(nint hWnd, int nCmdShow);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool SetWindowPos(nint hWnd, nint hWndInsertAfter, int x, int y, int cx, int cy, uint flags);
}
