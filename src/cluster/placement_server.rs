use std::net::SocketAddr;

use futures::{Future, Stream};
use log::{error, info};
use tokio::executor::DefaultExecutor;
use tokio::net::TcpListener;
use tower_grpc::{Code, Error, Request, Response, Status};
use tower_h2::client::Connection;
use tower_h2::{Body, Server};
use tower_http::AddOrigin;

use crate::cluster::consul_interface::NodeData;
use crate::cluster::placement::client::Placement;
use crate::cluster::placement::{server, PlacementReply, PlacementRequest};
use crate::cluster::shard::Shard;
use crate::cluster::ConsulInterface;

#[derive(Clone, Debug)]
pub struct Place {
    consul: ConsulInterface,
}

type PlacementFuture = Box<dyn Future<Item = Response<PlacementReply>, Error = Error> + Send + 'static>;

impl server::Placement for Place {
    type GetPlacementFuture = PlacementFuture;

    fn get_placement(&mut self, request: Request<PlacementRequest>) -> Self::GetPlacementFuture {
        info!("Request = {:?}", request);
        self.determine_placement(request)
    }
}

impl Place {
    pub fn determine_placement(&mut self, req: Request<PlacementRequest>) -> PlacementFuture {
        //        let index = req.get_ref().index.clone();
        //        let task = self
        //            .consul
        //            .get_index(index, true)
        //            .map_err(|err| Error::Grpc(Status::with_code_and_message(Code::Internal, err.to_string())))
        //            .and_then(move |c| {
        //                let kind = req.get_ref().kind.clone();
        //                let item: NodeData = c.get().skip(1).take(1).map(|k| k.Value.unwrap()).last().unwrap();
        //                let place = item.primaries.last().unwrap().shard_id().to_hyphenated().to_string();
        //
        //                Ok(Response::new(PlacementReply { node: place, kind }))
        //            });

        Box::new(futures::future::ok(Response::new(PlacementReply { node: "".into(), kind: 1 })))
    }

    pub fn get_service(addr: SocketAddr, consul: ConsulInterface) -> impl Future<Item = (), Error = ()> {
        let service = server::PlacementServer::new(Place { consul });
        let executor = DefaultExecutor::current();
        let mut h2 = Server::new(service, Default::default(), executor);

        info!("Binding on port: {:?}", addr);
        let bind = TcpListener::bind(&addr).unwrap_or_else(|_| panic!("Failed to bind to host: {:?}", addr));

        info!("Bound to: {:?}", &bind.local_addr().unwrap());
        bind.incoming()
            .for_each(move |sock| {
                let req = h2.serve(sock).map_err(|err| error!("h2 error: {:?}", err));
                tokio::spawn(req);
                Ok(())
            })
            .map_err(|err| error!("Server Error: {:?}", err))
    }

    pub fn create_client<C>(uri: http::Uri, conn: C) -> Placement<AddOrigin<C>> {
        use tower_http::add_origin;

        let conn: AddOrigin<C> = add_origin::Builder::new().uri(uri).build(conn).unwrap();
        Placement::new(conn)
    }
}

#[cfg(test)]
mod tests {
    use tokio::net::tcp::ConnectFuture;
    use tokio::net::TcpStream;
    use tower_h2::client::Connect;
    use tower_util::MakeService;

    use super::*;

    pub struct Conn(SocketAddr);

    impl tokio_connect::Connect for Conn {
        type Connected = TcpStream;
        type Error = std::io::Error;
        type Future = ConnectFuture;

        fn connect(&self) -> Self::Future {
            TcpStream::connect(&self.0)
        }
    }

    #[test]
    fn client_test() {
        let uri: http::Uri = format!("http://localhost:8081").parse().unwrap();
        let socket_addr: SocketAddr = "127.0.0.1:8081".parse().unwrap();
        let tcp_stream = Conn(socket_addr);

        let service = Place::get_service(socket_addr, ConsulInterface::default());
        let mut c = Connect::new(tcp_stream, Default::default(), DefaultExecutor::current());

        let place = c
            .make_service(())
            .map(move |conn| Place::create_client(uri, conn))
            .and_then(|mut client| {
                let req = Request::new(PlacementRequest {
                    index: "test".into(),
                    kind: 1,
                });

                client.get_placement(req).map_err(|e| panic!("gRPC request failed; err={:?}", e))
            })
            .map(|resp| println!("Response = {:#?}", resp))
            .map_err(|e| println!("ERROR = {:#?}", e));

        let s = service.select(place).map(|_| ()).map_err(|_| ());

        tokio::run(s);
    }
}
