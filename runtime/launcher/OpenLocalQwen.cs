using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Windows.Forms;

namespace LocalModelsLauncher
{
    internal static class Program
    {
        [STAThread]
        private static int Main(string[] args)
        {
            string root = AppDomain.CurrentDomain.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar);
            string script = Path.Combine(root, "scripts", "open-local-opencode.ps1");
            try
            {
                if (!File.Exists(script)) throw new FileNotFoundException("OpenCode launcher script is missing.", script);
                if (Has(args, "--check")) return RunCheck(root, script, args);
                string project = ValueAfter(args, "--project") ?? PickProject(root);
                if (project == null) return 0;
                project = Path.GetFullPath(project);
                if (!Directory.Exists(project)) throw new DirectoryNotFoundException(project);
                Remember(root, project);
                Process.Start(Interactive(script, project, args));
                return 0;
            }
            catch (Exception ex)
            {
                MessageBox.Show(ex.Message, "Open Local Qwen", MessageBoxButtons.OK, MessageBoxIcon.Error);
                return 1;
            }
        }

        private static ProcessStartInfo Interactive(string script, string project, string[] args)
        {
            string profile = ValueAfter(args, "--profile") ?? "stable-16k";
            string options = " -Profile " + Quote(profile);
            if (Has(args, "--vision")) options += " -WithVision";
            if (Has(args, "--lean")) options += " -Lean";
            if (Has(args, "--full-prompt")) options += " -FullPrompt";
            if (Has(args, "--plugins")) options += " -WithPlugins";
            if (Has(args, "--skills")) options += " -WithSkills";
            return new ProcessStartInfo
            {
                FileName = PowerShell(),
                Arguments = "-NoLogo -NoProfile -ExecutionPolicy Bypass -File " + Quote(script) + " -Project " + Quote(project) + options,
                WorkingDirectory = project,
                UseShellExecute = true,
                WindowStyle = ProcessWindowStyle.Normal
            };
        }

        private static int RunCheck(string root, string script, string[] args)
        {
            string profile = ValueAfter(args, "--profile") ?? "stable-16k";
            ProcessStartInfo start = new ProcessStartInfo
            {
                FileName = PowerShell(),
                Arguments = "-NoLogo -NoProfile -ExecutionPolicy Bypass -File " + Quote(script) + " -Project " + Quote(root) + " -Profile " + Quote(profile) + " -Check" + (Has(args, "--lean") ? " -Lean" : ""),
                WorkingDirectory = root,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            using (Process process = Process.Start(start))
            {
                string output = process.StandardOutput.ReadToEnd() + process.StandardError.ReadToEnd();
                process.WaitForExit();
                Directory.CreateDirectory(Path.Combine(root, "logs"));
                File.WriteAllText(Path.Combine(root, "logs", "launcher-check.log"), output, new UTF8Encoding(false));
                return process.ExitCode;
            }
        }

        private static string PickProject(string root)
        {
            Application.EnableVisualStyles();
            using (FolderBrowserDialog dialog = new FolderBrowserDialog())
            {
                dialog.Description = "Choose the project for local Qwen + OpenCode";
                dialog.SelectedPath = InitialProject(root);
                dialog.ShowNewFolderButton = false;
                return dialog.ShowDialog() == DialogResult.OK ? dialog.SelectedPath : null;
            }
        }

        private static string InitialProject(string root)
        {
            string state = Path.Combine(root, "config", "launcher-last-project.txt");
            if (File.Exists(state))
            {
                string saved = File.ReadAllText(state).Trim();
                if (Directory.Exists(saved)) return saved;
            }
            return Directory.Exists(Environment.CurrentDirectory) ? Environment.CurrentDirectory : root;
        }

        private static void Remember(string root, string project)
        {
            string path = Path.Combine(root, "config", "launcher-last-project.txt");
            Directory.CreateDirectory(Path.GetDirectoryName(path));
            File.WriteAllText(path, project, new UTF8Encoding(false));
        }

        private static bool Has(string[] args, string name)
        {
            foreach (string arg in args) if (string.Equals(arg, name, StringComparison.OrdinalIgnoreCase)) return true;
            return false;
        }

        private static string ValueAfter(string[] args, string name)
        {
            for (int i = 0; i + 1 < args.Length; i++)
                if (string.Equals(args[i], name, StringComparison.OrdinalIgnoreCase)) return args[i + 1];
            return null;
        }

        private static string PowerShell()
        {
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "WindowsPowerShell", "v1.0", "powershell.exe");
        }

        private static string Quote(string value) { return "\"" + value.Replace("\"", "\\\"") + "\""; }
    }
}
