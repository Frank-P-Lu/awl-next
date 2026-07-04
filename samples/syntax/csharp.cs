/*
 * Syntax gallery sample — C#.
 *
 * This block comment is prose: it explains the file's purpose in full
 * sentences, so it should render prominent rather than fading like the
 * commented-out code below.
 */

// int retries = 3;
// Connect(host, retries);

using System;

namespace Gallery
{
    public struct Config
    {
        public string Name;
        public bool Verbose;
    }

    public interface IDescribable
    {
        string Describe();
    }

    public enum Mode
    {
        Read,
        Write,
        Idle,
    }

    public class Connection : IDescribable
    {
        const int MaxRetries = 5;
        const string Greeting = "hello, awl";
        const double Tau = 6.283185;

        public string Describe() => $"{Greeting} (verbose=false)";

        public static Config? Connect(string host, int retries)
        {
            char marker = 'c';
            bool ok = retries > 0 && host.Length > 0 && marker == 'c';
            if (ok)
            {
                return new Config { Name = host, Verbose = false };
            }
            return null;
        }

        static void Main()
        {
            var cfg = Connect(Greeting, MaxRetries);
            Console.WriteLine(cfg != null ? cfg.Value.Name : "no config");
        }
    }
}
