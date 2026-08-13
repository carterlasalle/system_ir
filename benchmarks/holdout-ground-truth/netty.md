# netty
> https://github.com/netty/netty | Java | java lib (network) | ~693k LOC

## architecture
- transport — the channel/event-loop core (transport/src/main/java/io/netty/channel/)
- buffer — the ByteBuf abstraction (buffer/src/main/java/io/netty/buffer/)
- codec — codec framework: ByteToMessageDecoder, MessageToByteEncoder (codec/src/main/java/io/netty/handler/codec/)
- codec-http — HTTP/1.x codecs (codec-http/src/main/java/io/netty/handler/codec/http/)
- codec-http2 — HTTP/2 codecs (codec-http2/)
- handler — channel handlers: ssl, timeouts, logging (handler/)
- bootstrap — client/server bootstrap (transport/src/main/java/io/netty/bootstrap/)
- common — shared utilities: util, concurrent (common/)

## entrypoints
- ServerBootstrap — server bootstrap entry (transport/.../bootstrap/ServerBootstrap.java)
- Bootstrap — client bootstrap entry
- EventLoopGroup — thread pool abstraction
- NioEventLoopGroup — NIO event loop group
- ChannelInitializer — pipeline setup entry
- ChannelPipeline.addLast — handler registration
- EmbeddedChannel — in-memory channel for testing
- ByteBufAllocator — buffer allocation entry
- ChannelFuture — async operation handle
- ServerBootstrap.bind — bind server socket

## behavior
- ServerBootstrap.bind -> initAndRegister -> channel registration — server startup (ServerBootstrap.java)
- NioEventLoop.run -> select -> processSelectedKeys -> handle events — event loop (NioEventLoop.java)
- AbstractChannelHandlerContext.invokeChannelRead -> pipeline propagation — inbound flow (AbstractChannelHandlerContext.java)
- ChannelOutboundBuffer.write -> flush -> socket write — outbound flow
- ByteToMessageDecoder.channelRead -> decode loop -> fireChannelRead — decode pipeline (codec/ByteToMessageDecoder.java)
- EmbeddedChannel.writeInbound -> inbound processing — in-memory transport
- ChannelFuture.addListener -> promise completion — async completion

## state_authority
- Channel — the I/O channel state
- ChannelPipeline — ordered handler chain (DefaultChannelPipeline)
- ChannelHandlerContext — per-handler context state
- EventLoop — the event loop with task queue
- ByteBuf — reference-counted buffer state
- ChannelConfig — channel options state
- EventLoopGroup — the group's event loops
- Promise/ChannelPromise — completion state

## contracts
- new ServerBootstrap().group(bossGroup, workerGroup).channel(NioServerSocketChannel.class).childHandler(init).bind(port) — server bootstrap contract
- pipeline.addLast("decoder", new ByteToMessageDecoder(){...}) — pipeline registration contract
- channelRead(ctx, msg) — inbound handler contract
- write(ctx, msg) — outbound handler contract
- EmbeddedChannel(new Handler()).writeInbound(msg) — embedded test contract
- ByteBuf.alloc().buffer(n) — allocation contract
- channel.closeFuture().sync() — close contract
- NioEventLoopGroup(threads) — event loop group contract
- handler extends ChannelInboundHandlerAdapter — adapter base contract
- ChannelFuture.addListener(f) — async completion contract

## landmarks
- AbstractChannel — channel base class (transport/.../channel/AbstractChannel.java)
- AbstractChannelHandlerContext — handler context base
- ChannelInitializer — pipeline initializer
- ByteToMessageDecoder — decode adapter (codec/)
- MessageToByteEncoder — encode adapter
- NioEventLoop — NIO event loop
- DefaultChannelPipeline — pipeline implementation
- AbstractByteBuf — buffer base class (buffer/.../AbstractByteBuf.java)
- EmbeddedChannel — test channel

## tests
- transport/src/test/java/io/netty/channel/ — channel tests
- buffer/src/test/java/io/netty/buffer/ — buffer tests
- codec/src/test/java/io/netty/handler/codec/ — codec tests
- codec-http/src/test/java/io/netty/handler/codec/http/ — HTTP tests
- handler/src/test/java/io/netty/handler/ — handler tests
