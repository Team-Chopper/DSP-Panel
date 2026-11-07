import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

// Services
import 'services/server_connection.dart';

// Providers
import 'providers/panel_provider.dart';

// Pages
import 'pages/login_page.dart';
import 'pages/panel_page.dart';

// RTA
import 'services/rta_settings_provider.dart';

// Theme
import 'theme/app_theme.dart';

void main() {
  final server = ServerConnection();
  // Created before connecting so it receives the first "hello" message.
  final panel = PanelProvider(server);
  server.start();
  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: server),
        ChangeNotifierProvider.value(value: panel),
        ChangeNotifierProvider(create: (_) => RtaSettingsProvider()),
      ],
      child: const MyApp(),
    ),
  );
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'DSP Panel',
      debugShowCheckedModeBanner: false,
      theme: buildAppTheme(),
      home: const AppShell(),
    );
  }
}

/// Shows the login page or the panel, depending on the server state.
class AppShell extends StatelessWidget {
  const AppShell({super.key});

  @override
  Widget build(BuildContext context) {
    final state = context.select<ServerConnection, ServerState>((s) => s.state);
    switch (state) {
      case ServerState.checking:
        return const Scaffold(body: Center(child: CircularProgressIndicator()));
      case ServerState.loginRequired:
        return const LoginPage();
      case ServerState.connecting:
      case ServerState.connected:
      case ServerState.disconnected:
        return const PanelPage();
    }
  }
}
