from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.view_state import ViewState
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ClusterView")



@_attrs_define
class ClusterView:
    """ This node's view, as `/status`, the heartbeat record and the
    `system.cluster_view_changed` event carry it.

        Attributes:
            state (ViewState): What this node's view of the cluster looks like.
            summary (str): One sentence for a person.
            since (int | None | Unset): When the current state began (Unix seconds); absent while whole.
            lost_regions (list[str] | Unset): The regions whose every node went at once.
            view_age_secs (int | None | Unset): Seconds since discovery last worked — how old the member list is.
                Absent before the first discovery.
            database_reachable (bool | None | Unset): Whether the node reaches its database, when it has a probe and the
                view is not whole.
     """

    state: ViewState
    summary: str
    since: int | None | Unset = UNSET
    lost_regions: list[str] | Unset = UNSET
    view_age_secs: int | None | Unset = UNSET
    database_reachable: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        state = self.state.value

        summary = self.summary

        since: int | None | Unset
        if isinstance(self.since, Unset):
            since = UNSET
        else:
            since = self.since

        lost_regions: list[str] | Unset = UNSET
        if not isinstance(self.lost_regions, Unset):
            lost_regions = self.lost_regions



        view_age_secs: int | None | Unset
        if isinstance(self.view_age_secs, Unset):
            view_age_secs = UNSET
        else:
            view_age_secs = self.view_age_secs

        database_reachable: bool | None | Unset
        if isinstance(self.database_reachable, Unset):
            database_reachable = UNSET
        else:
            database_reachable = self.database_reachable


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "state": state,
            "summary": summary,
        })
        if since is not UNSET:
            field_dict["since"] = since
        if lost_regions is not UNSET:
            field_dict["lost_regions"] = lost_regions
        if view_age_secs is not UNSET:
            field_dict["view_age_secs"] = view_age_secs
        if database_reachable is not UNSET:
            field_dict["database_reachable"] = database_reachable

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        state = ViewState(d.pop("state"))




        summary = d.pop("summary")

        def _parse_since(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        since = _parse_since(d.pop("since", UNSET))


        lost_regions = cast(list[str], d.pop("lost_regions", UNSET))


        def _parse_view_age_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        view_age_secs = _parse_view_age_secs(d.pop("view_age_secs", UNSET))


        def _parse_database_reachable(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        database_reachable = _parse_database_reachable(d.pop("database_reachable", UNSET))


        cluster_view = cls(
            state=state,
            summary=summary,
            since=since,
            lost_regions=lost_regions,
            view_age_secs=view_age_secs,
            database_reachable=database_reachable,
        )


        cluster_view.additional_properties = d
        return cluster_view

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
