// Reads Dart files with the official analyzer (syntax only, no resolution) and prints what the
// wara-bridge code graph needs: directives, top-level symbols, references, navigation, routes and tokens.
//
// Input (stdin): JSON {"files": ["C:/abs/path/a.dart", ...]}
// Output (stdout): JSON {"<path>": {symbols, imports, refs, routes} | {"error": "..."}}
import 'dart:convert';
import 'dart:io';

import 'package:analyzer/dart/analysis/features.dart';
import 'package:analyzer/dart/analysis/utilities.dart';
import 'package:analyzer/dart/ast/ast.dart';
import 'package:analyzer/dart/ast/token.dart';
import 'package:analyzer/dart/ast/visitor.dart';

const widgetBases = {
  'StatelessWidget', 'StatefulWidget', 'InheritedWidget', 'InheritedNotifier', 'InheritedModel',
  'ConsumerWidget', 'ConsumerStatefulWidget', 'HookWidget', 'HookConsumerWidget', 'StatefulHookWidget',
  'StatefulHookConsumerWidget', 'GetView', 'GetWidget', 'RenderObjectWidget', 'LeafRenderObjectWidget',
  'SingleChildRenderObjectWidget', 'MultiChildRenderObjectWidget', 'ProxyWidget', 'ParentDataWidget',
  'ImplicitlyAnimatedWidget', 'AnimatedWidget',
};
// Widgets built inside these become screens (pushed routes) or overlays.
const routeBuilders = {'MaterialPageRoute', 'CupertinoPageRoute', 'PageRouteBuilder', 'DialogRoute', 'ModalBottomSheetRoute', 'MaterialPage', 'CupertinoPage'};
const overlayCalls = {'showDialog', 'showModalBottomSheet', 'showCupertinoModalPopup', 'showCupertinoDialog', 'showGeneralDialog', 'showBottomSheet', 'showSearch'};

final _tokenClass = RegExp(r'(Colou?rs?|Palette|Spacing|Spaces?|Radius|Radii|Sizes?|Shadows?|Elevations?|Typography|TextStyles?|Fonts?|Tokens?|Insets|Gaps|Durations|Breakpoints|Theme)$');

/// Classes that hold design tokens (AppColors, AppSpacing, Palette, Tokens…).
bool isTokenClass(String name) => _tokenClass.hasMatch(name);

void main() async {
  final input = jsonDecode(await stdin.transform(utf8.decoder).join()) as Map<String, dynamic>;
  final out = <String, Object?>{};
  for (final path in (input['files'] as List).cast<String>()) {
    try {
      out[path] = parseFile(path);
    } catch (e) {
      out[path] = {'error': e.toString().split('\n').first};
    }
  }
  stdout.write(jsonEncode(out));
}

/// Declared name of a declaration, tolerant of analyzer API changes between versions.
String? nameOf(AstNode d) {
  final dyn = d as dynamic;
  for (final read in <Object? Function()>[
    () => (dyn.name as Token?)?.lexeme,
    () => (dyn.namePart.typeName as Token?)?.lexeme,
    () => (dyn.primaryConstructor.typeName as Token?)?.lexeme,
  ]) {
    try {
      final v = read();
      if (v is String && v.isNotEmpty) return v;
    } catch (_) {}
  }
  // Fallback: the identifier right after the declaring keyword.
  for (Token? t = d.beginToken; t != null && t != d.endToken; t = t.next) {
    if (const {'class', 'mixin', 'enum', 'typedef', 'extension', 'type'}.contains(t.lexeme) && t.next?.type == TokenType.IDENTIFIER) {
      if (t.lexeme == 'extension' && t.next?.lexeme == 'type') continue;
      return t.next!.lexeme;
    }
  }
  return null;
}

/// Class/mixin members (older analyzers: node.members; newer: node.body.members).
NodeList<ClassMember> membersOfDecl(AstNode d) {
  final dyn = d as dynamic;
  try {
    return dyn.members as NodeList<ClassMember>;
  } catch (_) {
    return dyn.body.members as NodeList<ClassMember>;
  }
}

/// A named argument "label: expr" — NamedArgument in new analyzers, NamedExpression in older ones.
(String, Expression)? namedArg(AstNode a) {
  if (!a.runtimeType.toString().startsWith('Named')) return null;
  final d = a as dynamic;
  try {
    return ((d.name as Token).lexeme, d.argumentExpression as Expression);
  } catch (_) {}
  try {
    return (d.name.label.name as String, d.expression as Expression);
  } catch (_) {}
  return null;
}

/// Declared name of a formal parameter (wrapped in DefaultFormalParameter in older analyzers).
String? paramName(AstNode p) {
  final d = p as dynamic;
  try {
    final t = d.name as Token?;
    if (t != null) return t.lexeme;
  } catch (_) {}
  try {
    return (d.parameter.name as Token?)?.lexeme;
  } catch (_) {}
  return null;
}

/// "p.Foo<T>" → ("p", "Foo") ; "Foo" → (null, "Foo")
(String?, String) splitType(String source) {
  final bare = source.replaceAll(RegExp(r'<.*$', dotAll: true), '').replaceAll('?', '').trim();
  final dot = bare.lastIndexOf('.');
  return dot < 0 ? (null, bare) : (bare.substring(0, dot), bare.substring(dot + 1));
}

bool isPascal(String s) => RegExp(r'^_?[A-Z][A-Za-z0-9_]*$').hasMatch(s);

String clip(String s, int max) => s.length > max ? '${s.substring(0, max)}…' : s;

Map<String, Object?> parseFile(String path) {
  final result = parseString(
    content: File(path).readAsStringSync(),
    path: path,
    featureSet: FeatureSet.latestLanguageVersion(),
    throwIfDiagnostics: false,
  );
  final unit = result.unit;
  final lines = result.lineInfo;
  int lineOf(int offset) => lines.getLocation(offset).lineNumber;

  final imports = <Map<String, Object?>>[];
  for (final d in unit.directives) {
    final line = lineOf(d.offset);
    if (d is ImportDirective) {
      imports.add({'kind': 'import', 'source': d.uri.stringValue ?? '', 'imported': '*', 'local': d.prefix?.name, 'line': line});
    } else if (d is ExportDirective) {
      imports.add({'kind': 'reexport_all', 'source': d.uri.stringValue ?? '', 'imported': '*', 'local': null, 'line': line});
    } else if (d is PartDirective) {
      imports.add({'kind': 'part', 'source': d.uri.stringValue ?? '', 'imported': '*', 'local': null, 'line': line});
    } else if (d is PartOfDirective) {
      final uri = d.uri?.stringValue;
      if (uri != null) imports.add({'kind': 'part_of', 'source': uri, 'imported': '*', 'local': null, 'line': line});
    }
  }

  // ---- top-level symbols ----
  final symbols = <Map<String, Object?>>[];
  final owners = <AstNode, int>{}; // declaration node → symbol index that owns references inside it
  final classMembers = <int, Set<String>>{}; // symbol index → member names (fields/methods) of that class
  void add(AstNode node, String name, String kind, {Map<String, Object?>? props}) {
    owners[node] = symbols.length;
    symbols.add({'name': name, 'kind': kind, 'exported': name.startsWith('_') ? 0 : 1, 'isDefault': 0,
      'line': lineOf(node.offset), 'endLine': lineOf(node.end), 'start': node.offset, 'end': node.end, 'props': props});
  }

  Set<String> membersOf(NodeList<ClassMember> members) => {
    for (final m in members)
      if (m is FieldDeclaration) ...m.fields.variables.map((v) => v.name.lexeme)
      else if (m is MethodDeclaration) m.name.lexeme,
  };

  // static const/final fields with their values (design tokens live in classes like AppColors).
  List<Map<String, Object?>> staticsOf(NodeList<ClassMember> members) => [
    for (final m in members.whereType<FieldDeclaration>())
      if (m.isStatic)
        for (final v in m.fields.variables)
          {'name': v.name.lexeme, 'value': clip((v.initializer?.toSource() ?? '').replaceAll(RegExp(r'\s+'), ' '), 80), 'line': lineOf(v.offset)},
  ].take(300).toList();

  List<String> ctorParams(NodeList<ClassMember> members) {
    for (final m in members.whereType<ConstructorDeclaration>()) {
      if (m.name != null) continue; // the unnamed constructor describes the widget's inputs
      return [
        for (final p in m.parameters.parameters)
          if (!p.toSource().contains('super.'))
            (paramName(p) ?? '') + (p.toSource().contains('required') || (!p.toSource().contains('{') && !p.toSource().contains('=')) ? '' : '?'),
      ].where((s) => s.isNotEmpty && s != '?').toList();
    }
    return [];
  }

  final states = <ClassDeclaration, String>{}; // State<X> classes, merged into X below
  for (final d in unit.declarations) {
    if (d is ClassDeclaration) {
      final name = nameOf(d) ?? '?';
      final members = membersOfDecl(d);
      final superSrc = d.extendsClause?.superclass.toSource();
      final (_, superName) = superSrc == null ? (null, '') : splitType(superSrc);
      final stateOf = RegExp(r'^State<\s*([A-Za-z0-9_]+)').firstMatch(superSrc ?? '')?.group(1);
      if (stateOf != null) states[d] = stateOf;
      final kind = widgetBases.contains(superName) ? 'widget' : stateOf != null ? 'state' : 'class';
      final params = kind == 'widget' ? ctorParams(members) : const <String>[];
      final statics = kind == 'class' && isTokenClass(name) ? staticsOf(members) : const <Map<String, Object?>>[];
      add(d, name, kind, props: {
        if (superName.isNotEmpty) 'extends': superName,
        if (stateOf != null) 'of': stateOf,
        if (params.isNotEmpty) 'names': params,
        if (statics.isNotEmpty) 'statics': statics,
      });
      classMembers[owners[d]!] = membersOf(members);
    } else if (d is MixinDeclaration) {
      add(d, nameOf(d) ?? '?', 'class');
      classMembers[owners[d]!] = membersOf(membersOfDecl(d));
    } else if (d is ExtensionDeclaration) {
      final name = nameOf(d);
      if (name != null) add(d, name, 'class');
    } else if (d is EnumDeclaration) {
      add(d, nameOf(d) ?? '?', 'enum');
    } else if (d is TypeAlias) {
      add(d, nameOf(d) ?? '?', 'type');
    } else if (d is FunctionDeclaration) {
      add(d, d.name.lexeme, 'function');
    } else if (d is TopLevelVariableDeclaration) {
      for (final v in d.variables.variables) {
        owners[v] = symbols.length;
        symbols.add({'name': v.name.lexeme, 'kind': 'const', 'exported': v.name.lexeme.startsWith('_') ? 0 : 1, 'isDefault': 0,
          'line': lineOf(d.offset), 'endLine': lineOf(d.end), 'start': d.offset, 'end': d.end, 'props': null});
      }
    } else {
      final name = nameOf(d); // extension types and future declaration kinds
      if (name != null) add(d, name, 'class');
    }
  }
  // References inside a State class count for its widget (that is where build() lives).
  for (final MapEntry(key: cls, value: widget) in states.entries) {
    final idx = symbols.indexWhere((s) => s['name'] == widget);
    if (idx >= 0) {
      owners[cls] = idx;
      classMembers[idx] = {...?classMembers[idx], ...?classMembers[symbols.indexWhere((s) => s['name'] == nameOf(cls))]};
    }
  }

  // ---- references ----
  final topNames = {for (final s in symbols) s['name'] as String};
  // Names that mean "itself" for each owner: its own name and its State class (createState, constructors).
  final selfNames = <int, Set<String>>{};
  for (var i = 0; i < symbols.length; i++) {
    (selfNames[i] ??= {}).add(symbols[i]['name'] as String);
  }
  for (final MapEntry(key: cls, value: widget) in states.entries) {
    final idx = symbols.indexWhere((s) => s['name'] == widget);
    final name = nameOf(cls);
    if (idx >= 0 && name != null) (selfNames[idx] ??= {}).add(name);
  }
  final collector = _Refs(owners, classMembers, topNames, selfNames, lineOf);
  for (var i = 0; i < symbols.length; i++) {
    if (isTokenClass(symbols[i]['name'] as String)) collector.tokenClasses.add(i);
  }
  for (final d in unit.declarations) {
    d.accept(collector);
  }
  return {'symbols': symbols, 'imports': imports, 'refs': collector.refs.values.toList(), 'routes': collector.routes};
}

class _Refs extends RecursiveAstVisitor<void> {
  _Refs(this.owners, this.classMembers, this.topNames, this.selfNames, this.lineOf);
  final Map<AstNode, int> owners;
  final Map<int, Set<String>> classMembers;
  final Set<String> topNames;
  final Map<int, Set<String>> selfNames;
  final int Function(int) lineOf;
  final refs = <String, Map<String, Object?>>{};
  final routes = <Map<String, Object?>>[];
  final Set<int> tokenClasses = {};

  int owner = -1;
  Set<String> locals = {};
  bool tokenOwner = false; // inside a token class: its Color(...) values are definitions, not hardcoded colors
  String mode = 'jsx'; // how widget constructions are counted right now: jsx (renders) | nav | overlay

  void ref(String local, String? member, String kind, int offset) {
    if (local.isEmpty || locals.contains(local) || local == 'override') return;
    if (owner >= 0 && member == null && (selfNames[owner]?.contains(local) ?? false)) return;
    if (owner >= 0 && (classMembers[owner]?.contains(local) ?? false) && member == null && !topNames.contains(local)) return;
    final key = '$owner|$local|${member ?? ''}|$kind';
    final r = refs[key];
    if (r != null) {
      r['count'] = (r['count'] as int) + 1;
    } else {
      refs[key] = {'owner': owner, 'local': local, 'member': member, 'kind': kind, 'line': lineOf(offset), 'count': 1};
    }
  }

  // A construction like Foo(…), const Foo(…) or p.Foo(…).
  void construct(String? prefix, String name, int offset) {
    if (prefix != null) {
      ref(prefix, name, isPascal(name) ? mode : 'ref', offset);
    } else {
      ref(name, null, isPascal(name) ? mode : 'ref', offset);
    }
  }

  // Color(0xFF2563EB) written inline instead of a token.
  void hardcoded(String name, ArgumentList args, int offset) {
    if (name != 'Color' || tokenOwner || args.arguments.isEmpty) return;
    final a = args.arguments.first;
    if (a is IntegerLiteral) ref('#hex', a.literal.lexeme, 'hex', offset);
  }

  T within<T>(AstNode node, T Function() body) {
    final o = owners[node];
    if (o == null) return body();
    final savedOwner = owner, savedLocals = locals, savedToken = tokenOwner;
    owner = o;
    locals = {};
    tokenOwner = tokenClasses.contains(o);
    try {
      return body();
    } finally {
      owner = savedOwner;
      locals = savedLocals;
      tokenOwner = savedToken;
    }
  }

  @override
  void visitClassDeclaration(ClassDeclaration node) => within(node, () => super.visitClassDeclaration(node));
  @override
  void visitMixinDeclaration(MixinDeclaration node) => within(node, () => super.visitMixinDeclaration(node));
  @override
  void visitExtensionDeclaration(ExtensionDeclaration node) => within(node, () => super.visitExtensionDeclaration(node));
  @override
  void visitEnumDeclaration(EnumDeclaration node) => within(node, () => super.visitEnumDeclaration(node));
  @override
  void visitFunctionDeclaration(FunctionDeclaration node) => within(node, () => super.visitFunctionDeclaration(node));
  @override
  void visitVariableDeclaration(VariableDeclaration node) {
    if (owners.containsKey(node)) return within(node, () => super.visitVariableDeclaration(node));
    locals.add(node.name.lexeme);
    super.visitVariableDeclaration(node);
  }

  // Parameters and pattern variables are locals, not dependencies.
  @override
  void visitFormalParameterList(FormalParameterList node) {
    for (final p in node.parameters) {
      final n = paramName(p);
      if (n != null) locals.add(n);
    }
    super.visitFormalParameterList(node);
  }

  @override
  void visitDeclaredIdentifier(DeclaredIdentifier node) {
    locals.add(node.name.lexeme);
    super.visitDeclaredIdentifier(node);
  }

  @override
  void visitDeclaredVariablePattern(DeclaredVariablePattern node) {
    locals.add(node.name.lexeme);
    super.visitDeclaredVariablePattern(node);
  }

  @override
  void visitCatchClause(CatchClause node) {
    for (final p in [node.exceptionParameter, node.stackTraceParameter]) {
      if (p != null) locals.add(p.name.lexeme);
    }
    super.visitCatchClause(node);
  }

  @override
  void visitInstanceCreationExpression(InstanceCreationExpression node) {
    final (prefix, name) = splitType(node.constructorName.type.toSource());
    construct(prefix, name, node.offset);
    hardcoded(name, node.argumentList, node.offset);
    visitArgs(name, node.argumentList);
  }

  @override
  void visitMethodInvocation(MethodInvocation node) {
    final target = node.target;
    final name = node.methodName.name;
    if (target == null) {
      construct(null, name, node.offset);
      hardcoded(name, node.argumentList, node.offset);
    } else if (target is SimpleIdentifier && isPascal(name) && !isPascal(target.name)) {
      construct(target.name, name, node.offset); // prefixed constructor: p.Foo(…)
    } else if (target is SimpleIdentifier) {
      ref(target.name, isPascal(target.name) ? name : null, 'ref', node.offset); // Foo.named(…) / helper.call(…)
    } else {
      target.accept(this);
    }
    node.typeArguments?.accept(this);
    visitArgs(name, node.argumentList);
  }

  // Route/overlay builders change how the widgets built inside them are counted.
  void visitArgs(String callee, ArgumentList args) {
    final saved = mode;
    if (routeBuilders.contains(callee)) mode = 'nav';
    if (overlayCalls.contains(callee)) mode = 'overlay';
    if (callee == 'GoRoute' || callee.endsWith('App')) collectRoutes(callee, args);
    for (final a in args.arguments) {
      final named = namedArg(a);
      if (named != null) {
        final (label, expr) = named;
        final inner = mode;
        if ((label == 'home' && callee.endsWith('App')) || (callee == 'GoRoute' && (label == 'builder' || label == 'pageBuilder'))) mode = 'nav';
        expr.accept(this);
        mode = inner;
      } else {
        a.accept(this);
      }
    }
    mode = saved;
  }

  void collectRoutes(String callee, ArgumentList args) {
    String? widgetIn(Expression e) {
      String? found;
      e.accept(_FirstConstruction((n) => found ??= n));
      return found;
    }

    final named = {for (final a in args.arguments) if (namedArg(a) case (final label, final expr)) label: expr};
    if (callee == 'GoRoute') {
      final path = named['path'];
      final builder = named['builder'] ?? named['pageBuilder'];
      if (path is SimpleStringLiteral && builder != null) {
        routes.add({'path': path.value, 'widget': widgetIn(builder), 'kind': 'screen', 'line': lineOf(path.offset)});
      }
    } else {
      final home = named['home'];
      if (home != null) routes.add({'path': '/', 'widget': widgetIn(home), 'kind': 'home', 'line': lineOf(home.offset)});
      final map = named['routes'];
      if (map is SetOrMapLiteral) {
        for (final e in map.elements.whereType<MapLiteralEntry>()) {
          final key = e.key;
          if (key is SimpleStringLiteral) routes.add({'path': key.value, 'widget': widgetIn(e.value), 'kind': 'screen', 'line': lineOf(e.offset)});
        }
      }
    }
  }

  @override
  void visitNamedType(NamedType node) {
    final (prefix, name) = splitType(node.toSource());
    if (prefix != null) {
      ref(prefix, name, 'type', node.offset);
    } else {
      ref(name, null, 'type', node.offset);
    }
    node.typeArguments?.accept(this);
  }

  // AppColors.primary / prefix.helper — the member is kept so token usage can be found.
  @override
  void visitPrefixedIdentifier(PrefixedIdentifier node) {
    ref(node.prefix.name, node.identifier.name, 'ref', node.offset);
  }

  @override
  void visitSimpleIdentifier(SimpleIdentifier node) {
    if (node.inDeclarationContext() || node.parent is Label) return;
    final parent = node.parent;
    if (parent is PropertyAccess && parent.propertyName == node) return; // x.foo — only x matters
    ref(node.name, null, 'ref', node.offset);
  }
}

/// First widget constructed inside an expression (for route tables).
class _FirstConstruction extends RecursiveAstVisitor<void> {
  _FirstConstruction(this.found);
  final void Function(String) found;

  @override
  void visitInstanceCreationExpression(InstanceCreationExpression node) {
    found(splitType(node.constructorName.type.toSource()).$2);
  }

  @override
  void visitMethodInvocation(MethodInvocation node) {
    if (node.target == null && isPascal(node.methodName.name)) return found(node.methodName.name);
    super.visitMethodInvocation(node);
  }
}
