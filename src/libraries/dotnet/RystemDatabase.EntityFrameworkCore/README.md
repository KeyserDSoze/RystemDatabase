# RystemDatabase.EntityFrameworkCore

Future provider / DbContext integration tracked by [#29](https://github.com/KeyserDSoze/RystemDatabase/issues/29).
The provider must implement actual LINQ translation, change tracking, materialization, relationships and transaction semantics, then satisfy EF Core specification tests.

**Status: planned — this is not a functioning EF Core provider yet.** It will build on the .NET SDK. A true EF Core provider is substantially more than an extension method.
