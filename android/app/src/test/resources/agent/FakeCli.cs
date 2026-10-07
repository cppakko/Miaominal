using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Text;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;

// Local contract fixture only. No real CLI, credential store or external API.
public class FakeCli {
    public static void Main(string[] args) {
        try { Run(args); }
        catch (Exception error) {
            Console.Error.WriteLine(error.GetType().FullName);
            Console.Error.WriteLine(error.Message);
            Environment.ExitCode = 1;
        }
    }
    private static void Run(string[] args) {
        Console.InputEncoding = Encoding.UTF8;
        Console.OutputEncoding = new UTF8Encoding(false);
        var json = new JavaScriptSerializer();
        string endpoint, model, protocol;
        if (args[0] == "acp") {
            var config = json.Deserialize<Dictionary<string, object>>(Environment.GetEnvironmentVariable("OPENCODE_CONFIG_CONTENT"));
            var providers = (Dictionary<string, object>)config["provider"];
            var provider = (Dictionary<string, object>)providers["miaominal_phone"];
            var options = (Dictionary<string, object>)provider["options"];
            endpoint = (string)options["baseURL"];
            model = ((string)config["model"]).Substring("miaominal_phone/".Length);
            protocol = (string)provider["npm"] == "@ai-sdk/openai" ? "responses" : "chat/completions";
            if ((string)options["apiKey"] != "{env:MIAOMINAL_AI_API_KEY}") throw new Exception("Incorrect key source");
        } else {
            endpoint = null;
            model = null;
            protocol = "responses";
            foreach (var arg in args) {
                if (arg.StartsWith("model=")) model = json.Deserialize<string>(arg.Substring(6));
                var match = Regex.Match(arg, "base_url=(\"(?:\\\\.|[^\"])*\")");
                if (match.Success) endpoint = json.Deserialize<string>(match.Groups[1].Value);
            }
            if (endpoint == null || model == null) throw new Exception("Missing Codex overrides: " + json.Serialize(args));
        }
        var line = Console.ReadLine();
        if (line == null) throw new Exception("Missing protocol input");
        var rpc = json.Deserialize<Dictionary<string, object>>(line);
        var request = (HttpWebRequest)WebRequest.Create(endpoint + "/" + protocol);
        request.Method = "POST";
        request.Timeout = 10000;
        request.ContentType = "application/json";
        request.Headers["Authorization"] = "Bearer " + Environment.GetEnvironmentVariable("MIAOMINAL_AI_API_KEY");
        var body = Encoding.UTF8.GetBytes(json.Serialize(new Dictionary<string, object> { { "model", model }, { "input", "fixture" } }));
        request.ContentLength = body.Length;
        using (var stream = request.GetRequestStream()) stream.Write(body, 0, body.Length);
        using (var response = request.GetResponse()) { }
        Console.WriteLine(json.Serialize(new Dictionary<string, object> {
            { "jsonrpc", "2.0" }, { "id", rpc["id"] },
            { "result", new Dictionary<string, object> { { "arguments", args }, { "model", model },
                { "processId", System.Diagnostics.Process.GetCurrentProcess().Id } } }
        }));
        if (rpc.ContainsKey("hold") && (bool)rpc["hold"]) System.Threading.Thread.Sleep(System.Threading.Timeout.Infinite);
    }
}
