import 'package:flutter_test/flutter_test.dart';
import 'package:tunes4r_player_example/main.dart';

void main() {
  test('formatMs formats milliseconds as m:ss', () {
    expect(formatMs(0), '0:00');
    expect(formatMs(1000), '0:01');
    expect(formatMs(59000), '0:59');
    expect(formatMs(60000), '1:00');
    expect(formatMs(61000), '1:01');
    expect(formatMs(3599000), '59:59');
  });
}
