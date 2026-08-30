import 'dart:convert';
import 'dart:io';

final class BookAgentClient {
  BookAgentClient({required this.baseUrl, required this.apiKey, HttpClient? httpClient})
      : _httpClient = httpClient ?? HttpClient();

  final Uri baseUrl;
  final String apiKey;
  final HttpClient _httpClient;

  Future<Map<String, Object?>> explain({
    required String bookId,
    required String selectedText,
    required Map<String, Object?> sourceRef,
    required Map<String, Object?> readerState,
  }) => _post('/v1/books/$bookId/explain', {
        'selected_text': selectedText,
        'source_ref': sourceRef,
        'reader_state': readerState,
        'spoiler_mode': 'read_range',
        'intent': 'explain',
      });

  Future<Map<String, Object?>> ask({
    required String bookId,
    required String question,
    required Map<String, Object?> readerState,
  }) => _post('/v1/books/$bookId/ask', {
        'question': question,
        'reader_state': readerState,
        'spoiler_mode': 'read_range',
      });

  Future<Map<String, Object?>> checkpoint({
    required String bookId,
    required String chapterId,
    required Map<String, Object?> readerState,
  }) => _post('/v1/books/$bookId/chapters/$chapterId/checkpoint', {
        'reader_state': readerState,
        'spoiler_mode': 'read_range',
      });

  Future<Map<String, Object?>> reflect({
    required String bookId,
    required String chapterId,
    required String checkpointId,
    required String questionId,
    required String answer,
    required Map<String, Object?> readerState,
  }) => _post('/v1/books/$bookId/chapters/$chapterId/reflect', {
        'checkpoint_id': checkpointId,
        'question_id': questionId,
        'answer': answer,
        'reader_state': readerState,
      });

  Future<Map<String, Object?>> _post(String path, Map<String, Object?> body) async {
    final request = await _httpClient.postUrl(baseUrl.resolve(path));
    request.headers
      ..set(HttpHeaders.authorizationHeader, 'Bearer $apiKey')
      ..contentType = ContentType.json;
    request.write(jsonEncode(body));
    final response = await request.close();
    final payload = jsonDecode(await utf8.decodeStream(response)) as Map<String, Object?>;
    if (response.statusCode < 200 || response.statusCode >= 300) {
      final error = payload['error'] as Map<String, Object?>?;
      throw HttpException(error?['message'] as String? ?? 'Codexia HTTP ${response.statusCode}');
    }
    return payload;
  }

  void close() => _httpClient.close(force: true);
}
