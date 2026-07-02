import 'dart:ui' as ui;

import 'package:flutter/material.dart';

// ---------------------------------------------------------------------------
// Enums & config types
// ---------------------------------------------------------------------------

enum ThumbStyle { circle, rect, diamond, image }

enum SliderMode { file, streaming, live }

/// Controls the visual appearance of the slider thumb.
class ThumbConfig {
  final ThumbStyle style;
  final double size;
  final double borderRadius;
  final ImageProvider? image;
  final ui.Image? resolvedImage;

  const ThumbConfig({
    this.style = ThumbStyle.circle,
    this.size = 16.0,
    this.borderRadius = 4.0,
    this.image,
    this.resolvedImage,
  });
}

// ---------------------------------------------------------------------------
// Track shape
// ---------------------------------------------------------------------------

/// A [SliderTrackShape] that draws an optional buffer fill between the thumb
/// and [bufferEndRatio] (0–1 relative to the full track width).
class BufferedSliderTrack extends SliderTrackShape {
  final double? bufferEndRatio;

  const BufferedSliderTrack({this.bufferEndRatio});

  @override
  Rect getPreferredRect({
    required RenderBox parentBox,
    Offset offset = Offset.zero,
    required SliderThemeData sliderTheme,
    bool isEnabled = false,
    bool isDiscrete = false,
  }) {
    final double trackHeight = sliderTheme.trackHeight ?? 4.0;
    final double trackLeft = offset.dx;
    final double trackTop =
        offset.dy + (parentBox.size.height - trackHeight) / 2;
    final double trackWidth = parentBox.size.width;
    return Rect.fromLTWH(trackLeft, trackTop, trackWidth, trackHeight);
  }

  @override
  void paint(
    PaintingContext context,
    Offset offset, {
    required RenderBox parentBox,
    required SliderThemeData sliderTheme,
    required Animation<double> enableAnimation,
    required Offset thumbCenter,
    Offset? secondaryOffset,
    bool isEnabled = false,
    bool isDiscrete = false,
    required TextDirection textDirection,
  }) {
    final Rect trackRect = getPreferredRect(
      parentBox: parentBox,
      offset: offset,
      sliderTheme: sliderTheme,
      isEnabled: isEnabled,
      isDiscrete: isDiscrete,
    );

    final double positionX =
        thumbCenter.dx.clamp(trackRect.left, trackRect.right);
    final double trackRadius = trackRect.height / 2;

    final Color inactiveColor = ColorTween(
      begin: sliderTheme.disabledInactiveTrackColor,
      end: sliderTheme.inactiveTrackColor,
    ).evaluate(enableAnimation)!;

    // 1. Full inactive track
    context.canvas.drawRRect(
      RRect.fromRectAndRadius(trackRect, Radius.circular(trackRadius)),
      Paint()..color = inactiveColor,
    );

    // 2. Buffer region (thumb → buffer end)
    final double? ratio = bufferEndRatio;
    if (ratio != null && ratio > 0) {
      final double bufferEndX =
          (trackRect.left + ratio * trackRect.width)
              .clamp(trackRect.left, trackRect.right);
      if (bufferEndX > positionX + 0.5) {
        context.canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTRB(
                positionX, trackRect.top, bufferEndX, trackRect.bottom),
            Radius.circular(trackRadius),
          ),
          Paint()
            ..color =
                sliderTheme.activeTrackColor!.withValues(alpha: 0.25),
        );
      }
    }

    // 3. Played (active) region
    final Color activeColor = ColorTween(
      begin: sliderTheme.disabledThumbColor,
      end: sliderTheme.activeTrackColor,
    ).evaluate(enableAnimation)!;
    context.canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTRB(
            trackRect.left, trackRect.top, positionX, trackRect.bottom),
        Radius.circular(trackRadius),
      ),
      Paint()..color = activeColor,
    );
  }
}

// ---------------------------------------------------------------------------
// Thumb shape
// ---------------------------------------------------------------------------

/// A [SliderComponentShape] that supports circle, rect, diamond, and image
/// thumb styles with a subtle press-scale animation.
class ConfigurableThumbShape extends SliderComponentShape {
  final ThumbConfig config;

  const ConfigurableThumbShape({required this.config});

  @override
  Size getPreferredSize(bool isEnabled, bool isDiscrete) =>
      Size(config.size, config.size);

  @override
  void paint(
    PaintingContext context,
    Offset center, {
    required Animation<double> activationAnimation,
    required Animation<double> enableAnimation,
    required bool isDiscrete,
    required TextPainter labelPainter,
    required RenderBox parentBox,
    required SliderThemeData sliderTheme,
    required TextDirection textDirection,
    required double value,
    required double textScaleFactor,
    required Size sizeWithOverflow,
  }) {
    final Canvas canvas = context.canvas;
    final double radius = config.size / 2;
    final double pressScale = 1.0 + 0.15 * activationAnimation.value;

    canvas.save();
    canvas.translate(center.dx, center.dy);
    canvas.scale(pressScale, pressScale);

    final Color thumbColor = ColorTween(
      begin: sliderTheme.disabledThumbColor,
      end: sliderTheme.thumbColor,
    ).evaluate(enableAnimation)!;

    final Paint fill = Paint()
      ..color = thumbColor
      ..style = PaintingStyle.fill;
    final Paint stroke = Paint()
      ..color = sliderTheme.activeTrackColor!.withValues(alpha: 0.5)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;

    switch (config.style) {
      case ThumbStyle.circle:
        canvas.drawCircle(Offset.zero, radius, fill);
        canvas.drawCircle(Offset.zero, radius, stroke);
      case ThumbStyle.rect:
        final rrect = RRect.fromRectAndRadius(
          Rect.fromCenter(
              center: Offset.zero,
              width: config.size,
              height: config.size),
          Radius.circular(config.borderRadius),
        );
        canvas.drawRRect(rrect, fill);
        canvas.drawRRect(rrect, stroke);
      case ThumbStyle.diamond:
        final path = Path()
          ..moveTo(0, -radius)
          ..lineTo(radius, 0)
          ..lineTo(0, radius)
          ..lineTo(-radius, 0)
          ..close();
        canvas.drawPath(path, fill);
        canvas.drawPath(path, stroke);
      case ThumbStyle.image:
        if (config.resolvedImage != null) {
          canvas.clipRRect(RRect.fromRectAndRadius(
            Rect.fromCircle(center: Offset.zero, radius: radius),
            Radius.circular(radius),
          ));
          canvas.drawImageRect(
            config.resolvedImage!,
            Rect.fromLTWH(
              0,
              0,
              config.resolvedImage!.width.toDouble(),
              config.resolvedImage!.height.toDouble(),
            ),
            Rect.fromCircle(center: Offset.zero, radius: radius),
            Paint(),
          );
        } else {
          canvas.drawCircle(Offset.zero, radius, fill);
          final tp = TextPainter(
            text: TextSpan(
              text: '\u266A',
              style: TextStyle(color: Colors.white, fontSize: radius),
            ),
            textDirection: TextDirection.ltr,
          )..layout();
          tp.paint(canvas, Offset(-tp.width / 2, -tp.height / 2));
        }
    }

    canvas.restore();
  }
}

// ---------------------------------------------------------------------------
// Live badge (pulsing)
// ---------------------------------------------------------------------------

class _LiveBadge extends StatefulWidget {
  const _LiveBadge();

  @override
  State<_LiveBadge> createState() => _LiveBadgeState();
}

class _LiveBadgeState extends State<_LiveBadge>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctrl;
  late final Animation<double> _opacity;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 900),
    )..repeat(reverse: true);
    _opacity = Tween<double>(begin: 0.55, end: 1.0).animate(
      CurvedAnimation(parent: _ctrl, curve: Curves.easeInOut),
    );
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _opacity,
      builder: (_, _) => Opacity(
        opacity: _opacity.value,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
          decoration: BoxDecoration(
            color: Colors.red,
            borderRadius: BorderRadius.circular(4),
          ),
          child: const Text(
            'LIVE',
            style: TextStyle(
              color: Colors.white,
              fontSize: 10,
              fontWeight: FontWeight.bold,
              letterSpacing: 0.5,
            ),
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Main widget
// ---------------------------------------------------------------------------

/// A seekable progress slider that supports file, streaming, and live modes.
///
/// - [file]: standard seek bar with optional buffer fill.
/// - [streaming]: seek bar where [bufferedToMs] indicates how far ahead has
///   been downloaded.
/// - [live]: DVR-style bar over a rolling [bufferCapacityMs] window; shows a
///   pulsing LIVE badge when at the edge, and a `-M:SS` offset label when
///   behind.
class ProgressSlider extends StatelessWidget {
  /// Current playback position in milliseconds.
  final int positionMs;

  /// Total duration in milliseconds. Used in [SliderMode.file] and
  /// [SliderMode.streaming]. May be 0 for live streams.
  final int durationMs;

  /// How far the buffer extends, in milliseconds.
  final int bufferedToMs;

  /// Rolling window size for live DVR mode, in milliseconds.
  final int bufferCapacityMs;

  final SliderMode mode;
  final ThumbConfig thumbConfig;

  /// Whether the user can interact with the slider.
  final bool enabled;

  final ValueChanged<double> onSeekChange;
  final ValueChanged<double> onSeekEnd;

  const ProgressSlider({
    super.key,
    required this.positionMs,
    required this.durationMs,
    this.bufferedToMs = 0,
    this.bufferCapacityMs = 0,
    this.mode = SliderMode.file,
    this.thumbConfig = const ThumbConfig(),
    this.enabled = true,
    required this.onSeekChange,
    required this.onSeekEnd,
  });

  // ---- helpers -------------------------------------------------------------

  /// Formats milliseconds as `H:MM:SS` (hours omitted when < 1 hour).
  static String _fmtMs(int ms) {
    if (ms < 0) ms = 0;
    final int totalSec = ms ~/ 1000;
    final int h = totalSec ~/ 3600;
    final int m = (totalSec % 3600) ~/ 60;
    final int s = totalSec % 60;
    final String mm = m.toString().padLeft(h > 0 ? 2 : 1, '0');
    final String ss = s.toString().padLeft(2, '0');
    return h > 0 ? '$h:$mm:$ss' : '$mm:$ss';
  }

  bool get _isLive => mode == SliderMode.live;

  /// `true` when within 2 s of the live edge.
  bool get _isAtLiveEdge =>
      _isLive &&
      bufferedToMs > 0 &&
      (bufferedToMs - positionMs).abs() < 2000;

  int get _effectiveDur {
    if (_isLive) return bufferCapacityMs;
    if (durationMs > 0) return durationMs;
    if (bufferedToMs > 0) return bufferedToMs;
    return 0;
  }

  double get _effectiveMax =>
      _effectiveDur > 0 ? _effectiveDur.toDouble() : 1.0;

  double get _clampedPosition =>
      positionMs.clamp(0, _effectiveDur).toDouble();

  double get _bufferEndRatio {
    final double max = _effectiveMax;
    final int bufMs =
        _isLive ? bufferedToMs.clamp(0, bufferCapacityMs) : bufferedToMs;
    return bufMs / max;
  }

  // ---- sub-builders --------------------------------------------------------

  Widget _buildTimeLabel(BuildContext context) {
    final TextStyle style = Theme.of(context).textTheme.bodySmall!;

    // In live mode behind the edge, show DVR offset (e.g. "-0:42").
    final String posLabel = _isLive && !_isAtLiveEdge
        ? '-${_fmtMs((bufferedToMs - positionMs).abs())}'
        : _fmtMs(positionMs);

    final String durLabel = _fmtMs(_effectiveDur);

    return Row(
      children: [
        Text(
          _isLive ? posLabel : '$posLabel / $durLabel',
          style: style,
        ),
        if (_isAtLiveEdge) ...[
          const SizedBox(width: 8),
          const _LiveBadge(),
        ],
      ],
    );
  }

  Widget _buildSlider(BuildContext context) {
    return SliderTheme(
      data: SliderTheme.of(context).copyWith(
        trackShape: BufferedSliderTrack(bufferEndRatio: _bufferEndRatio),
        thumbShape: ConfigurableThumbShape(config: thumbConfig),
        trackHeight: 4,
      ),
      child: Slider(
        value: _clampedPosition,
        max: _effectiveMax,
        onChanged: enabled ? onSeekChange : null,
        onChangeEnd: enabled ? onSeekEnd : null,
      ),
    );
  }

  // ---- build ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final bool hasRange = _effectiveDur > 0;

    final Widget content = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        _buildTimeLabel(context),
        if (hasRange) _buildSlider(context),
      ],
    );

    // Fade the whole widget when disabled, preserving layout.
    return Semantics(
      label: 'Playback position: ${_fmtMs(positionMs)}'
          ' of ${_fmtMs(_effectiveDur)}',
      slider: true,
      child: enabled
          ? content
          : Opacity(opacity: 0.4, child: content),
    );
  }
}
