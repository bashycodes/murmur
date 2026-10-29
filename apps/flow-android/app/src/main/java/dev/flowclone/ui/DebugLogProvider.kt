package dev.flowclone.ui

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import dev.flowclone.FlowApp

/**
 * D21 "Share / Export": serves exactly one file, `files/debug/dictations.jsonl`, read-only, to
 * the app the user picked in the share sheet (not exported; access only through the per-share
 * read grant). No androidx dependency, so a 40-line FileProvider instead of the library one.
 */
class DebugLogProvider : ContentProvider() {
    companion object {
        const val NAME = "dictations.jsonl"
        fun authority(c: Context) = "${c.packageName}.debuglog"
        fun uri(c: Context): Uri = Uri.parse("content://${authority(c)}/$NAME")

        /** User-initiated only (the Developer page button). */
        fun shareIntent(c: Context): Intent {
            val u = uri(c)
            val send = Intent(Intent.ACTION_SEND).apply {
                type = "text/plain"
                putExtra(Intent.EXTRA_STREAM, u)
                putExtra(Intent.EXTRA_SUBJECT, "Murmur debug log")
                clipData = android.content.ClipData.newRawUri(NAME, u)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            return Intent.createChooser(send, "Share debug log (includes transcripts)")
        }
    }

    private fun file() = FlowApp.of(context!!).debugFile.file

    private fun check(uri: Uri) = require(uri.lastPathSegment == NAME) { "unknown file" }

    override fun onCreate() = true

    override fun getType(uri: Uri) = "text/plain"

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        check(uri)
        require(mode == "r") { "read-only" }
        return ParcelFileDescriptor.open(file(), ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?): Cursor {
        check(uri)
        val cols = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val f = file()
        return MatrixCursor(cols, 1).apply {
            addRow(cols.map { if (it == OpenableColumns.SIZE) f.length() else if (it == OpenableColumns.DISPLAY_NAME) NAME else null })
        }
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?) = 0
}
