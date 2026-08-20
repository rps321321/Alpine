using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace LocalModelsLauncher
{
    internal static class Program
    {
        [STAThread]
        private static int Main(string[] args)
        {
            string root = AppDomain.CurrentDomain.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar);
            string alpine = Path.Combine(root, "alpine.exe");
            try
            {
                if (!File.Exists(alpine)) throw new FileNotFoundException("The installed Alpine control plane is missing. Re-run setup.", alpine);
                if (Has(args, "--check")) return RunCheck(root, alpine, args);
                string project = ValueAfter(args, "--project") ?? PickProject(root);
                if (project == null) return 0;
                project = Path.GetFullPath(project);
                if (!Directory.Exists(project)) throw new DirectoryNotFoundException(project);
                Remember(root, project);
                return RunInteractive(root, alpine, project, args);
            }
            catch (Exception ex)
            {
                string failureLog = Path.Combine(root, "logs", "launcher-last-error.log");
                try { failureLog = RecordAdapterFailure(root, args, ex); }
                catch { }
                string message = "The launcher adapter could not start Alpine (" + ex.GetType().Name + ")." +
                    Environment.NewLine + "Failure log: " + failureLog;
                if (DialogsEnabled())
                {
                    MessageBox.Show(message, "Open Local Qwen", MessageBoxButtons.OK, MessageBoxIcon.Error);
                }
                else
                {
                    Console.Error.WriteLine(message);
                }
                return 1;
            }
        }

        private static int RunInteractive(string root, string alpine, string project, string[] args)
        {
            string launchId = Guid.NewGuid().ToString("N");
            using (Process process = Process.Start(Interactive(root, alpine, project, args, launchId)))
            {
                process.WaitForExit();
                int exitCode = process.ExitCode;
                if (exitCode == 0) return 0;

                string logs = Path.Combine(root, "logs");
                string invocationLog = Path.Combine(logs, "launcher-errors", launchId + ".log");
                string failureLog = Path.Combine(logs, "launcher-last-error.log");
                string details;
                try
                {
                    details = File.Exists(invocationLog)
                        ? File.ReadAllText(invocationLog)
                        : "The PowerShell supervisor exited with code " + exitCode +
                          " without producing its per-launch failure record.";
                }
                catch (Exception readError)
                {
                    details = "The per-launch failure record could not be read: " + readError.Message;
                }

                if (DialogsEnabled())
                {
                    if (details.Length > 2000) details = details.Substring(0, 2000) + Environment.NewLine + "...";
                    MessageBox.Show(
                        "OpenCode failed to start or exited with an error." + Environment.NewLine + Environment.NewLine +
                        details + Environment.NewLine + "Failure log: " + failureLog,
                        "Open Local Qwen",
                        MessageBoxButtons.OK,
                        MessageBoxIcon.Error);
                }
                return exitCode;
            }
        }

        private static ProcessStartInfo Interactive(string root, string alpine, string project, string[] args, string launchId)
        {
            string profile = ValueAfter(args, "--profile") ?? "stable-16k";
            string options = "opencode --install-root " + Quote(root) + " --project " + Quote(project) +
                " --profile " + Quote(profile) + " --launch-id " + Quote(launchId) + " --allow-legacy-identity";
            if (Has(args, "--vision")) options += " --vision";
            if (Has(args, "--lean")) options += " --lean";
            if (Has(args, "--full-prompt")) options += " --full-prompt";
            if (Has(args, "--plugins")) options += " --plugins";
            if (Has(args, "--skills")) options += " --skills";
            if (Has(args, "--diagnostic-failure")) options += " --diagnostic-failure";
            return new ProcessStartInfo
            {
                FileName = alpine,
                Arguments = options,
                WorkingDirectory = project,
                UseShellExecute = true,
                WindowStyle = ProcessWindowStyle.Normal
            };
        }

        private static int RunCheck(string root, string alpine, string[] args)
        {
            string profile = ValueAfter(args, "--profile") ?? "stable-16k";
            string launchId = Guid.NewGuid().ToString("N");
            ProcessStartInfo start = new ProcessStartInfo
            {
                FileName = alpine,
                Arguments = "opencode --install-root " + Quote(root) + " --project " + Quote(root) + " --profile " + Quote(profile) + " --launch-id " + Quote(launchId) + " --check" + (Has(args, "--lean") ? " --lean" : ""),
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
            string temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(path));
                File.WriteAllText(temporary, project, new UTF8Encoding(false));
                if (File.Exists(path))
                {
                    File.Replace(temporary, path, null);
                }
                else
                {
                    File.Move(temporary, path);
                }
            }
            catch (IOException)
            {
                // This is a convenience hint, not launch-critical state. Concurrent
                // launchers are allowed to race and whichever publication wins is valid.
            }
            catch (UnauthorizedAccessException)
            {
                // A read-only installation must still be able to launch an explicit project.
            }
            finally
            {
                try
                {
                    if (File.Exists(temporary)) File.Delete(temporary);
                }
                catch (IOException) { }
                catch (UnauthorizedAccessException) { }
            }
        }

        private static bool DialogsEnabled()
        {
            return !string.Equals(
                Environment.GetEnvironmentVariable("LOCALMODEL_LAUNCHER_NO_DIALOG"),
                "1",
                StringComparison.Ordinal);
        }

        private static string RecordAdapterFailure(string root, string[] args, Exception error)
        {
            string launchId = Guid.NewGuid().ToString("N");
            string logs = Path.Combine(root, "logs");
            string directory = Path.Combine(logs, "launcher-errors");
            string invocation = Path.Combine(directory, launchId + ".log");
            string stable = Path.Combine(logs, "launcher-last-error.log");
            Directory.CreateDirectory(directory);
            string content = "timestamp=" + DateTime.UtcNow.ToString("o") + Environment.NewLine +
                "launch_id=" + launchId + Environment.NewLine +
                "profile=" + (ValueAfter(args, "--profile") ?? "stable-16k") + Environment.NewLine +
                "error:" + Environment.NewLine +
                "The launcher adapter could not start Alpine (" + error.GetType().Name + "). Re-run setup." + Environment.NewLine;
            WriteAtomic(invocation, content);
            using (Mutex mutex = new Mutex(false, @"Local\OpenLocalQwenAdapterFailureLog"))
            {
                bool acquired = false;
                try
                {
                    try { acquired = mutex.WaitOne(TimeSpan.FromSeconds(5)); }
                    catch (AbandonedMutexException) { acquired = true; }
                    if (acquired) WriteAtomic(stable, content);
                }
                finally { if (acquired) mutex.ReleaseMutex(); }
            }
            return stable;
        }

        private static void WriteAtomic(string path, string content)
        {
            string temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
            string backup = path + "." + Guid.NewGuid().ToString("N") + ".bak";
            try
            {
                File.WriteAllText(temporary, content, new UTF8Encoding(false));
                if (File.Exists(path)) File.Replace(temporary, path, backup);
                else File.Move(temporary, path);
            }
            finally
            {
                if (File.Exists(temporary)) File.Delete(temporary);
                if (File.Exists(backup)) File.Delete(backup);
            }
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

        private static string Quote(string value) { return "\"" + value.Replace("\"", "\\\"") + "\""; }
    }
}
