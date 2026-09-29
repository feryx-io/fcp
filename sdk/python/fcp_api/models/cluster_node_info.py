from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.cluster_node_status import ClusterNodeStatus
from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.cluster_view import ClusterView
  from ..models.node_capacity import NodeCapacity
  from ..models.video_device_budget import VideoDeviceBudget





T = TypeVar("T", bound="ClusterNodeInfo")



@_attrs_define
class ClusterNodeInfo:
    """ Information about a single cluster node, enriched with derived fields.

        Attributes:
            node_id (str):
            node_type (str):
            http_address (str):
            sip_address (str):
            capabilities (list[str]):
            active_calls (int):
            started_at (datetime.datetime): When the node started, as RFC 3339 in the admin API
                (`docs/DEVELOPER_SURFACE.md` §3, phase 2b); epoch seconds here.
            version (str):
            status (ClusterNodeStatus): Status of a cluster node, derived from its Redis key TTL.
            uptime_seconds (int):
            draining (bool): Shutting down; taking nothing new.
            region (None | str | Unset): Deployment region the node advertises, when it has one.
            video_budget (list[VideoDeviceBudget] | Unset): Video budget per device, when the node advertises one.
            capacity (NodeCapacity | None | Unset):
            calls_left (int | None | Unset): Calls this node will still take, when it says what it allows.
            view (ClusterView | None | Unset):
     """

    node_id: str
    node_type: str
    http_address: str
    sip_address: str
    capabilities: list[str]
    active_calls: int
    started_at: datetime.datetime
    version: str
    status: ClusterNodeStatus
    uptime_seconds: int
    draining: bool
    region: None | str | Unset = UNSET
    video_budget: list[VideoDeviceBudget] | Unset = UNSET
    capacity: NodeCapacity | None | Unset = UNSET
    calls_left: int | None | Unset = UNSET
    view: ClusterView | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.cluster_view import ClusterView # noqa: PLC0415
        from ..models.node_capacity import NodeCapacity # noqa: PLC0415
        from ..models.video_device_budget import VideoDeviceBudget # noqa: PLC0415
        node_id = self.node_id

        node_type = self.node_type

        http_address = self.http_address

        sip_address = self.sip_address

        capabilities = self.capabilities



        active_calls = self.active_calls

        started_at = self.started_at.isoformat()

        version = self.version

        status = self.status.value

        uptime_seconds = self.uptime_seconds

        draining = self.draining

        region: None | str | Unset
        if isinstance(self.region, Unset):
            region = UNSET
        else:
            region = self.region

        video_budget: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.video_budget, Unset):
            video_budget = []
            for video_budget_item_data in self.video_budget:
                video_budget_item = video_budget_item_data.to_dict()
                video_budget.append(video_budget_item)



        capacity: dict[str, Any] | None | Unset
        if isinstance(self.capacity, Unset):
            capacity = UNSET
        elif isinstance(self.capacity, NodeCapacity):
            capacity = self.capacity.to_dict()
        else:
            capacity = self.capacity

        calls_left: int | None | Unset
        if isinstance(self.calls_left, Unset):
            calls_left = UNSET
        else:
            calls_left = self.calls_left

        view: dict[str, Any] | None | Unset
        if isinstance(self.view, Unset):
            view = UNSET
        elif isinstance(self.view, ClusterView):
            view = self.view.to_dict()
        else:
            view = self.view


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "node_id": node_id,
            "node_type": node_type,
            "http_address": http_address,
            "sip_address": sip_address,
            "capabilities": capabilities,
            "active_calls": active_calls,
            "started_at": started_at,
            "version": version,
            "status": status,
            "uptime_seconds": uptime_seconds,
            "draining": draining,
        })
        if region is not UNSET:
            field_dict["region"] = region
        if video_budget is not UNSET:
            field_dict["video_budget"] = video_budget
        if capacity is not UNSET:
            field_dict["capacity"] = capacity
        if calls_left is not UNSET:
            field_dict["calls_left"] = calls_left
        if view is not UNSET:
            field_dict["view"] = view

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.cluster_view import ClusterView # noqa: PLC0415
        from ..models.node_capacity import NodeCapacity # noqa: PLC0415
        from ..models.video_device_budget import VideoDeviceBudget # noqa: PLC0415
        d = dict(src_dict)
        node_id = d.pop("node_id")

        node_type = d.pop("node_type")

        http_address = d.pop("http_address")

        sip_address = d.pop("sip_address")

        capabilities = cast(list[str], d.pop("capabilities"))


        active_calls = d.pop("active_calls")

        started_at = datetime.datetime.fromisoformat(d.pop("started_at"))




        version = d.pop("version")

        status = ClusterNodeStatus(d.pop("status"))




        uptime_seconds = d.pop("uptime_seconds")

        draining = d.pop("draining")

        def _parse_region(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        region = _parse_region(d.pop("region", UNSET))


        _video_budget = d.pop("video_budget", UNSET)
        video_budget: list[VideoDeviceBudget] | Unset = UNSET
        if _video_budget is not UNSET:
            video_budget = []
            for video_budget_item_data in _video_budget:
                video_budget_item = VideoDeviceBudget.from_dict(video_budget_item_data)



                video_budget.append(video_budget_item)


        def _parse_capacity(data: object) -> NodeCapacity | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                capacity_type_0 = NodeCapacity.from_dict(data)



                return capacity_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(NodeCapacity | None | Unset, data)

        capacity = _parse_capacity(d.pop("capacity", UNSET))


        def _parse_calls_left(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        calls_left = _parse_calls_left(d.pop("calls_left", UNSET))


        def _parse_view(data: object) -> ClusterView | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                view_type_0 = ClusterView.from_dict(data)



                return view_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ClusterView | None | Unset, data)

        view = _parse_view(d.pop("view", UNSET))


        cluster_node_info = cls(
            node_id=node_id,
            node_type=node_type,
            http_address=http_address,
            sip_address=sip_address,
            capabilities=capabilities,
            active_calls=active_calls,
            started_at=started_at,
            version=version,
            status=status,
            uptime_seconds=uptime_seconds,
            draining=draining,
            region=region,
            video_budget=video_budget,
            capacity=capacity,
            calls_left=calls_left,
            view=view,
        )


        cluster_node_info.additional_properties = d
        return cluster_node_info

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
