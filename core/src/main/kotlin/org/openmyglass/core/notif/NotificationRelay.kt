package org.openmyglass.core.notif

import com.google.protobuf.ByteString
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.openmyglass.protocol.v1.Bridge
import org.openmyglass.protocol.v1.Bridge.Frame

/**
 * Keeps Glass's notification list equal to the phone's filtered set: a notification is sent when it is new or
 * its content changed, a removal only for notifications Glass has, app icons once per session, and on attach the
 * whole current set (without waking: those are not new). Actions chosen on Glass come back through [onAction].
 */
class NotificationRelay(
    private val filter: () -> NotificationFilter,
    private val icons: (sha256Hex: String) -> ByteArray?,
    private val onAction: suspend (key: String, request: ActionRequest) -> Unit,
) {
    sealed interface ActionRequest {
        data class Fire(val index: Int) : ActionRequest
        data object Dismiss : ActionRequest
    }

    class Session internal constructor(internal val send: suspend (Frame) -> Unit, internal val onFailure: () -> Unit)

    private val lock = Mutex()
    private val items = LinkedHashMap<String, NotificationItem>()   // accepted, in posting order
    private val sent = HashMap<String, Bridge.Notification>()       // what the active session has
    private val sentIcons = HashSet<String>()
    private var active: Session? = null

    suspend fun posted(item: NotificationItem) = lock.withLock {
        val f = filter()
        if (!f.accept(item)) {
            if (items.remove(item.key) != null) sendRemoval(item.key)
            return@withLock
        }
        val isNew = item.key !in items
        items[item.key] = item
        send(item, wake = isNew && f.wake(item))
    }

    suspend fun removed(key: String) = lock.withLock {
        if (items.remove(key) != null) sendRemoval(key)
    }

    /** Removes every notification (e.g. the owner turned notifications off). */
    suspend fun clear() = lock.withLock {
        for (key in items.keys.toList()) {
            items.remove(key)
            sendRemoval(key)
        }
    }

    /** Starts a session (replacing any previous one) and sends the current set. See NavigationRelay.attach. */
    suspend fun attach(onFailure: () -> Unit = {}, send: suspend (Frame) -> Unit): Session = lock.withLock {
        val session = Session(send, onFailure)
        active = session
        sent.clear()
        sentIcons.clear()
        for (item in items.values.toList()) send(item, wake = false)
        session
    }

    suspend fun detach(session: Session) = lock.withLock {
        if (active === session) active = null
    }

    suspend fun onIconRequest(sha256: ByteArray) = lock.withLock {
        val hex = sha256.toHex()
        icons(hex)?.let { png -> if (deliver(iconFrame(hex, png))) sentIcons += hex }
        Unit
    }

    /** An action chosen on Glass; ignored if the notification is already gone (removed on the phone meanwhile). */
    suspend fun onAction(action: Bridge.NotificationAction) {
        val known = lock.withLock { action.key in items }
        if (!known) return
        val request = when (action.choiceCase) {
            Bridge.NotificationAction.ChoiceCase.ACTION_INDEX -> ActionRequest.Fire(action.actionIndex)
            Bridge.NotificationAction.ChoiceCase.DISMISS -> ActionRequest.Dismiss
            else -> return
        }
        onAction(action.key, request)   // outside the lock: the callback may post or remove notifications
    }

    private suspend fun send(item: NotificationItem, wake: Boolean) {
        val proto = item.toProto(wake)
        if (sent[item.key]?.toBuilder()?.setWake(wake)?.build() == proto) return   // same content: nothing to do
        item.iconSha256?.let { hex ->
            if (hex !in sentIcons) icons(hex)?.let { png -> if (deliver(iconFrame(hex, png))) sentIcons += hex }
        }
        if (deliver(Frame.newBuilder().setNotification(Bridge.NotificationPosted.newBuilder().setNotification(proto)).build())) {
            sent[item.key] = proto
        }
    }

    private suspend fun sendRemoval(key: String) {
        if (sent.remove(key) != null) deliver(Frame.newBuilder().setNotificationRemoved(Bridge.NotificationRemoved.newBuilder().setKey(key)).build())
    }

    /** Like NavigationRelay.deliver: a failed send ends the session and tells its owner to close the link. */
    private suspend fun deliver(frame: Frame): Boolean {
        val session = active ?: return false
        return try {
            session.send(frame)
            true
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            if (active === session) active = null
            sent.clear()
            runCatching { session.onFailure() }
            false
        }
    }

    private fun iconFrame(hex: String, png: ByteArray) = Frame.newBuilder().setIcon(
        Bridge.Icon.newBuilder().setSha256(ByteString.copyFrom(hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()))
            .setPng(ByteString.copyFrom(png))
    ).build()

    private fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }
}
