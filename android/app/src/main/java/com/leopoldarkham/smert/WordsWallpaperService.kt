package com.leopoldarkham.smert

import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
import android.service.wallpaper.WallpaperService
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import android.text.TextUtils
import android.view.SurfaceHolder
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.time.Instant
import java.time.OffsetDateTime
import java.util.concurrent.Executors

private const val API_URL = "http://dx63m4krbosa1k4t6nitmay6.167.233.56.221.sslip.io/api/wotd"

/** Refetch at most this often when the screen turns on, to pick up rerolls. */
private const val MIN_REFRESH_MS = 15 * 60 * 1000L

private val BACKGROUND = Color.parseColor("#21222d")
private val INK = Color.parseColor("#fff6ec")
private val MUTED = Color.parseColor("#7a7870")
private val SOFT = Color.parseColor("#c4bdb5")

/** Draws today's French and English words from smert's /api/wotd. */
class WordsWallpaperService : WallpaperService() {

    override fun onCreateEngine(): Engine = WordsEngine()

    private inner class WordsEngine : Engine() {
        private val handler = Handler(Looper.getMainLooper())
        private val io = Executors.newSingleThreadExecutor()
        private val prefs = getSharedPreferences("wotd", Context.MODE_PRIVATE)
        private val keyguard = getSystemService(KeyguardManager::class.java)
        private var fetching = false

        private val refreshAtRollover = Runnable { fetch() }

        // Lock state changes don't always change wallpaper visibility, so
        // redraw on them explicitly.
        private val screenEvents = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action == Intent.ACTION_SCREEN_ON) refreshIfStale()
                draw()
            }
        }

        override fun onCreate(surfaceHolder: SurfaceHolder) {
            super.onCreate(surfaceHolder)
            val filter = IntentFilter().apply {
                addAction(Intent.ACTION_SCREEN_ON)
                addAction(Intent.ACTION_SCREEN_OFF)
                addAction(Intent.ACTION_USER_PRESENT)
            }
            registerReceiver(screenEvents, filter, Context.RECEIVER_EXPORTED)
            scheduleRollover()
        }

        override fun onDestroy() {
            unregisterReceiver(screenEvents)
            handler.removeCallbacksAndMessages(null)
            io.shutdown()
            super.onDestroy()
        }

        override fun onVisibilityChanged(visible: Boolean) {
            if (visible) {
                refreshIfStale()
                draw()
            }
        }

        override fun onSurfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
            super.onSurfaceChanged(holder, format, width, height)
            draw()
        }

        // ── Data ──────────────────────────────────────────────────────────────

        private fun cached(): JSONObject? =
            prefs.getString("json", null)?.let { runCatching { JSONObject(it) }.getOrNull() }

        private fun nextRollover(json: JSONObject?): Instant? =
            json?.optString("next_rollover")?.takeIf { it.isNotEmpty() }
                ?.let { runCatching { OffsetDateTime.parse(it).toInstant() }.getOrNull() }

        private fun refreshIfStale() {
            val json = cached()
            val fetchedAt = prefs.getLong("fetched_at", 0)
            val rolledOver = nextRollover(json)?.let { !Instant.now().isBefore(it) } ?: true
            if (json == null || rolledOver || System.currentTimeMillis() - fetchedAt > MIN_REFRESH_MS) {
                fetch()
            }
        }

        private fun fetch() {
            if (fetching) return
            fetching = true
            io.execute {
                val body = runCatching {
                    val conn = URL(API_URL).openConnection() as HttpURLConnection
                    conn.connectTimeout = 10_000
                    conn.readTimeout = 10_000
                    try {
                        conn.inputStream.bufferedReader().use { it.readText() }
                    } finally {
                        conn.disconnect()
                    }
                }.getOrNull()?.takeIf { runCatching { JSONObject(it) }.isSuccess }

                handler.post {
                    fetching = false
                    if (body != null) {
                        prefs.edit()
                            .putString("json", body)
                            .putLong("fetched_at", System.currentTimeMillis())
                            .apply()
                    }
                    scheduleRollover()
                    draw()
                }
            }
        }

        /** Fetch again a minute after the next rollover, while this engine lives. */
        private fun scheduleRollover() {
            handler.removeCallbacks(refreshAtRollover)
            val at = nextRollover(cached()) ?: return
            val delay = at.toEpochMilli() - System.currentTimeMillis() + 60_000
            handler.postDelayed(refreshAtRollover, delay.coerceAtLeast(60_000))
        }

        // ── Drawing ───────────────────────────────────────────────────────────

        private fun draw() {
            val holder = surfaceHolder
            val canvas = runCatching { holder.lockCanvas() }.getOrNull() ?: return
            try {
                render(canvas)
            } finally {
                holder.unlockCanvasAndPost(canvas)
            }
        }

        private fun render(canvas: Canvas) {
            val w = canvas.width.toFloat()
            val h = canvas.height.toFloat()
            canvas.drawColor(BACKGROUND)

            val json = cached()
            val margin = w * 0.09f
            val textWidth = (w - 2 * margin).toInt()
            var y = h * 0.42f

            if (json == null) {
                y = drawText(canvas, "Loading today's words…", serif(w * 0.04f, Typeface.ITALIC, MUTED), margin, y, textWidth, 2)
            } else {
                for ((key, label) in listOf("french" to "FRANÇAIS", "english" to "ENGLISH")) {
                    val word = json.optJSONObject(key) ?: continue
                    y = drawText(canvas, label, sans(w * 0.026f, MUTED, letterSpacing = 0.15f), margin, y, textWidth, 1)
                    y += w * 0.01f
                    y = drawText(canvas, word.optString("word"), serif(w * 0.075f, Typeface.NORMAL, INK), margin, y, textWidth, 1)
                    val translation = word.optString("translation")
                    if (translation.isNotEmpty()) {
                        y = drawText(canvas, translation, serif(w * 0.04f, Typeface.ITALIC, SOFT), margin, y, textWidth, 1)
                    }
                    y += w * 0.015f
                    val definition = word.optString("definition").lines().firstOrNull { it.isNotBlank() } ?: ""
                    y = drawText(canvas, definition, serif(w * 0.036f, Typeface.NORMAL, SOFT), margin, y, textWidth, 4)
                    y += w * 0.08f
                }
            }

            // Diagnostic for this prototype: does Android report the lock state?
            val locked = keyguard.isKeyguardLocked
            drawText(canvas, "prototype · locked=$locked", sans(w * 0.022f, MUTED), margin, h * 0.95f, textWidth, 1)
        }

        /** Draws wrapped text at (x, top), returning the y just below it. */
        private fun drawText(
            canvas: Canvas, text: String, paint: TextPaint,
            x: Float, top: Float, width: Int, maxLines: Int,
        ): Float {
            val layout = StaticLayout.Builder.obtain(text, 0, text.length, paint, width)
                .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                .setMaxLines(maxLines)
                .setEllipsize(TextUtils.TruncateAt.END)
                .setLineSpacing(0f, 1.15f)
                .build()
            canvas.save()
            canvas.translate(x, top)
            layout.draw(canvas)
            canvas.restore()
            return top + layout.height
        }

        private fun serif(size: Float, style: Int, color: Int) = TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply {
            textSize = size
            typeface = Typeface.create(Typeface.SERIF, style)
            this.color = color
        }

        private fun sans(size: Float, color: Int, letterSpacing: Float = 0f) = TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply {
            textSize = size
            typeface = Typeface.SANS_SERIF
            this.color = color
            this.letterSpacing = letterSpacing
        }
    }
}
