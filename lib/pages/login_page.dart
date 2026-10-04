// Login page (only shown when the server has login enabled)

import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../services/server_connection.dart';

class LoginPage extends StatefulWidget {
  const LoginPage({super.key});

  @override
  State<LoginPage> createState() => _LoginPageState();
}

class _LoginPageState extends State<LoginPage> {
  final _password = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _password.dispose();
    super.dispose();
  }

  Future<void> _login() async {
    if (_password.text.isEmpty || _busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    final error = await context.read<ServerConnection>().login(_password.text);
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = error;
    });
    if (error != null) _password.clear();
  }

  @override
  Widget build(BuildContext context) {
    final server = context.watch<ServerConnection>();
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 360),
          child: Card(
            margin: const EdgeInsets.all(16),
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: AutofillGroup(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text('DSP Panel',
                        style: Theme.of(context).textTheme.headlineSmall,
                        textAlign: TextAlign.center),
                    const SizedBox(height: 24),
                    if (!server.passwordSet)
                      const Padding(
                        padding: EdgeInsets.only(bottom: 16),
                        child: Text(
                          'No password has been set on the server yet.\n'
                          'Run "dsp-server passwd" on the server first.',
                          textAlign: TextAlign.center,
                        ),
                      ),
                    TextField(
                      controller: _password,
                      obscureText: true,
                      autofocus: true,
                      enabled: !_busy,
                      autofillHints: const [AutofillHints.password],
                      decoration: InputDecoration(labelText: 'Password', errorText: _error),
                      onSubmitted: (_) => _login(),
                    ),
                    const SizedBox(height: 16),
                    ElevatedButton(
                      onPressed: _busy ? null : _login,
                      child: _busy
                          ? const SizedBox(
                              width: 18, height: 18, child: CircularProgressIndicator(strokeWidth: 2))
                          : const Text('Log in'),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
