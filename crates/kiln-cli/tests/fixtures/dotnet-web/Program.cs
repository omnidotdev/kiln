// A minimal ASP.NET app. The aspnet:9.0 runtime image listens on port 8080 by
// default. Verifies dotnet publish -> aspnet runtime -> `dotnet app.dll`.
var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();
app.MapGet("/", () => "ok\n");
app.Run();
