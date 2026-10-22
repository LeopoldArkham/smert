package com.leopoldarkham.smert

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

/**
 * The free band of the lock screen, as fractions of its height: below the
 * clock and date, above the owner message and the shortcut buttons.
 */
private const val AREA_TOP = 0.40f
private const val AREA_BOTTOM = 0.84f

/** Text shrinks down to this fraction of its normal size to fit. */
private const val MIN_SCALE = 0.55f

private val LANGUAGES = listOf("french" to "FRANÇAIS", "english" to "ENGLISH")

/**
 * Lock-screen wallpaper showing one of smert's two daily words, with its full
 * definition, from /api/wotd. The language alternates each time the screen
 * turns on.
 */
class WordsWallpaperService : WallpaperService() {

    override fun onCreateEngine(): Engine = WordsEngine()

    private inner class WordsEngine : Engine() {
        private val handler = Handler(Looper.getMainLooper())
        private val io = Executors.newSingleThreadExecutor()
        private val prefs = getSharedPreferences("wotd", Context.MODE_PRIVATE)
        private val serif = resources.getFont(R.font.baskervville)
        private val serifItalic = resources.getFont(R.font.baskervville_italic)
        private var fetching = false

        private val refreshAtRollover = Runnable { fetch() }

        private val screenOn = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                val next = (prefs.getInt("language", 0) + 1) % LANGUAGES.size
                prefs.edit().putInt("language", next).apply()
                refreshIfStale()
                draw()
            }
        }

        override fun onCreate(surfaceHolder: SurfaceHolder) {
            super.onCreate(surfaceHolder)
            registerReceiver(screenOn, IntentFilter(Intent.ACTION_SCREEN_ON), Context.RECEIVER_EXPORTED)
            scheduleRollover()
        }

        override fun onDestroy() {
            unregisterReceiver(screenOn)
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

            val margin = (w * 0.09f).toInt()
            val width = w.toInt() - 2 * margin
            val top = h * AREA_TOP
            val bottom = h * AREA_BOTTOM

            val json = cached()
            val entry = json?.let { pickWord(it) }
            val blocks = if (entry == null) {
                listOf(Block("Loading today's words…", sizeFor(w, 0.04f), serifItalic, MUTED))
            } else {
                wordBlocks(entry.first, entry.second, w)
            }

            // Shrink until everything fits the free band; at the smallest size,
            // cut the definition short instead.
            var scale = 1f
            var layouts = layout(blocks, width, scale, Int.MAX_VALUE)
            while (height(layouts, scale) > bottom - top && scale > MIN_SCALE) {
                scale -= 0.05f
                layouts = layout(blocks, width, scale, Int.MAX_VALUE)
            }
            if (height(layouts, scale) > bottom - top) {
                layouts = fitByTruncating(blocks, width, scale, bottom - top)
            }

            // Centre the text in the free band.
            var y = top + ((bottom - top) - height(layouts, scale)) / 2
            for ((block, layout) in layouts) {
                y += block.spaceBefore * scale
                canvas.save()
                canvas.translate(margin.toFloat(), y)
                layout.draw(canvas)
                canvas.restore()
                y += layout.height
            }
        }

        /** Today's word in the current language, or the other one if missing. */
        private fun pickWord(json: JSONObject): Pair<String, JSONObject>? {
            val start = prefs.getInt("language", 0) % LANGUAGES.size
            for (i in LANGUAGES.indices) {
                val (key, label) = LANGUAGES[(start + i) % LANGUAGES.size]
                json.optJSONObject(key)?.let { return label to it }
            }
            return null
        }

        private fun wordBlocks(label: String, word: JSONObject, w: Float): List<Block> {
            val blocks = mutableListOf(
                Block(label, sizeFor(w, 0.026f), Typeface.SANS_SERIF, MUTED, letterSpacing = 0.15f),
                Block(word.optString("word"), sizeFor(w, 0.085f), serif, INK, spaceBefore = w * 0.01f),
            )
            word.optString("translation").takeIf { it.isNotBlank() }?.let {
                blocks += Block(it, sizeFor(w, 0.045f), serifItalic, SOFT)
            }
            // One paragraph per meaning, numbered when there are several.
            val meanings = word.optString("definition").lines().map { it.trim() }.filter { it.isNotEmpty() }
            meanings.forEachIndexed { i, meaning ->
                val text = if (meanings.size > 1) "${i + 1}. $meaning" else meaning
                blocks += Block(
                    text, sizeFor(w, 0.042f), serif, SOFT,
                    spaceBefore = if (i == 0) w * 0.04f else w * 0.02f,
                    isDefinition = true,
                )
            }
            return blocks
        }

        private fun sizeFor(w: Float, fraction: Float) = w * fraction

        /**
         * At the smallest scale and still too tall: give the definition lines
         * what space is left and end the last one visible with an ellipsis.
         */
        private fun fitByTruncating(
            blocks: List<Block>, width: Int, scale: Float, available: Float,
        ): List<Pair<Block, StaticLayout>> {
            val result = mutableListOf<Pair<Block, StaticLayout>>()
            var used = 0f
            for (block in blocks) {
                val full = buildLayout(block, width, scale, Int.MAX_VALUE)
                val space = block.spaceBefore * scale
                if (!block.isDefinition) {
                    result += block to full
                    used += space + full.height
                    continue
                }
                val lineHeight = full.height.toFloat() / full.lineCount
                val linesLeft = ((available - used - space) / lineHeight).toInt()
                if (linesLeft <= 0) break
                val layout = buildLayout(block, width, scale, linesLeft)
                result += block to layout
                used += space + layout.height
                if (linesLeft < full.lineCount) break
            }
            return result
        }

        private fun layout(blocks: List<Block>, width: Int, scale: Float, maxLines: Int) =
            blocks.map { it to buildLayout(it, width, scale, maxLines) }

        private fun height(layouts: List<Pair<Block, StaticLayout>>, scale: Float) =
            layouts.sumOf { (block, layout) -> (block.spaceBefore * scale + layout.height).toDouble() }.toFloat()

        private fun buildLayout(block: Block, width: Int, scale: Float, maxLines: Int): StaticLayout {
            val paint = TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply {
                textSize = block.size * scale
                typeface = block.typeface
                color = block.color
                letterSpacing = block.letterSpacing
            }
            return StaticLayout.Builder.obtain(block.text, 0, block.text.length, paint, width)
                .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                .setLineSpacing(0f, 1.15f)
                .setMaxLines(maxLines)
                .setEllipsize(TextUtils.TruncateAt.END)
                .build()
        }
    }

    private class Block(
        val text: String,
        val size: Float,
        val typeface: Typeface,
        val color: Int,
        val letterSpacing: Float = 0f,
        val spaceBefore: Float = 0f,
        val isDefinition: Boolean = false,
    )
}
