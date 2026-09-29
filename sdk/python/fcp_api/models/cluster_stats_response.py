from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.cluster_stats import ClusterStats





T = TypeVar("T", bound="ClusterStatsResponse")



@_attrs_define
class ClusterStatsResponse:
    """ Response for GET /api/v1/cluster/stats

        Attributes:
            available (bool):
            message (None | str | Unset):
            stats (ClusterStats | None | Unset):
     """

    available: bool
    message: None | str | Unset = UNSET
    stats: ClusterStats | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.cluster_stats import ClusterStats # noqa: PLC0415
        available = self.available

        message: None | str | Unset
        if isinstance(self.message, Unset):
            message = UNSET
        else:
            message = self.message

        stats: dict[str, Any] | None | Unset
        if isinstance(self.stats, Unset):
            stats = UNSET
        elif isinstance(self.stats, ClusterStats):
            stats = self.stats.to_dict()
        else:
            stats = self.stats


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "available": available,
        })
        if message is not UNSET:
            field_dict["message"] = message
        if stats is not UNSET:
            field_dict["stats"] = stats

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.cluster_stats import ClusterStats # noqa: PLC0415
        d = dict(src_dict)
        available = d.pop("available")

        def _parse_message(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        message = _parse_message(d.pop("message", UNSET))


        def _parse_stats(data: object) -> ClusterStats | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                stats_type_0 = ClusterStats.from_dict(data)



                return stats_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ClusterStats | None | Unset, data)

        stats = _parse_stats(d.pop("stats", UNSET))


        cluster_stats_response = cls(
            available=available,
            message=message,
            stats=stats,
        )


        cluster_stats_response.additional_properties = d
        return cluster_stats_response

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
