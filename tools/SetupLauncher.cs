using System;
using System.Collections.Generic;
using System.IO;
using System.Management.Automation;
using System.Management.Automation.Runspaces;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Windows.Forms;

[assembly: AssemblyTitle("BetterExplorer Setup")]
[assembly: AssemblyProduct("BetterExplorer")]

internal static class SetupLauncher
{
    private const string Title = "BetterExplorer Setup";
    private const uint AttachParentProcess = 0xffffffff;

    [DllImport("kernel32.dll")]
    private static extern bool AttachConsole(uint processId);

    [STAThread]
    private static int Main(string[] args)
    {
        bool interactive = args.Length == 0;
        if (!interactive)
            AttachConsole(AttachParentProcess);

        try
        {
            bool uninstall;
            var parameters = ParseArguments(args, out uninstall);
            RunEmbeddedScript(uninstall, parameters, interactive);
            return 0;
        }
        catch (Exception error)
        {
            if (interactive)
                MessageBox.Show(error.Message, Title, MessageBoxButtons.OK, MessageBoxIcon.Error);
            else
                Console.Error.WriteLine(error.Message);
            return 1;
        }
    }

    private static Dictionary<string, object> ParseArguments(string[] args, out bool uninstall)
    {
        var parameters = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        uninstall = false;
        for (int index = 0; index < args.Length; index++)
        {
            string argument = args[index];
            string name = argument.TrimStart('-');
            switch (argument.ToLowerInvariant())
            {
                case "--uninstall":
                    if (uninstall)
                        throw new ArgumentException("Specify --uninstall once.");
                    uninstall = true;
                    break;
                case "-quiet":
                case "-startmenushortcut":
                    parameters.Add(name, true);
                    break;
                case "-wine":
                case "-testroot":
                    if (index + 1 == args.Length || args[index + 1].StartsWith("-", StringComparison.Ordinal))
                        throw new ArgumentException("A value is required for " + argument + ".");
                    parameters.Add(name, args[++index]);
                    break;
                default:
                    throw new ArgumentException("Unknown setup argument " + argument + ".");
            }
        }
        return parameters;
    }

    private static void RunEmbeddedScript(bool uninstall, Dictionary<string, object> parameters, bool interactive)
    {
        string resource = uninstall ? "BetterExplorer.Uninstall.ps1" : "BetterExplorer.Install.ps1";
        using (Stream source = Assembly.GetExecutingAssembly().GetManifestResourceStream(resource))
        {
            if (source == null)
                throw new InvalidOperationException("The setup program is missing its embedded script.");

            using (var reader = new StreamReader(source))
            using (Runspace runspace = RunspaceFactory.CreateRunspace())
            using (PowerShell shell = PowerShell.Create())
            {
                runspace.ApartmentState = System.Threading.ApartmentState.STA;
                runspace.ThreadOptions = PSThreadOptions.UseCurrentThread;
                runspace.Open();
                runspace.SessionStateProxy.SetVariable("BetterExplorerBundleRoot", AppDomain.CurrentDomain.BaseDirectory);
                runspace.SessionStateProxy.SetVariable("BetterExplorerHostedSetup", true);
                shell.Runspace = runspace;
                shell.AddScript(reader.ReadToEnd(), true);
                foreach (var parameter in parameters)
                    shell.AddParameter(parameter.Key, parameter.Value);

                shell.Invoke();
                if (shell.Streams.Error.Count > 0)
                    throw new InvalidOperationException(shell.Streams.Error[0].ToString());

                if (!interactive)
                {
                    foreach (InformationRecord message in shell.Streams.Information)
                        Console.WriteLine(message.MessageData);
                    foreach (WarningRecord warning in shell.Streams.Warning)
                        Console.Error.WriteLine(warning.Message);
                }
            }
        }
    }
}
