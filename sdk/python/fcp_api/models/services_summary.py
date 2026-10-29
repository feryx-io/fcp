from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="ServicesSummary")



@_attrs_define
class ServicesSummary:
    """ Every service in one number each, for the dashboard.

        Attributes:
            total (int):
            serving (int): Serving.
            attention (int): Degraded or draining.
            down (int): Down or stopped.
            unknown (int):
            redis_reachable (bool): Whether this admin can read the platform's Redis now.
            redis_error (None | str | Unset): Why it cannot, when it cannot.
            read_at (datetime.datetime | None | Unset): When the beacons were last read.
     """

    total: int
    serving: int
    attention: int
    down: int
    unknown: int
    redis_reachable: bool
    redis_error: None | str | Unset = UNSET
    read_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        total = self.total

        serving = self.serving

        attention = self.attention

        down = self.down

        unknown = self.unknown

        redis_reachable = self.redis_reachable

        redis_error: None | str | Unset
        if isinstance(self.redis_error, Unset):
            redis_error = UNSET
        else:
            redis_error = self.redis_error

        read_at: None | str | Unset
        if isinstance(self.read_at, Unset):
            read_at = UNSET
        elif isinstance(self.read_at, datetime.datetime):
            read_at = self.read_at.isoformat()
        else:
            read_at = self.read_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "total": total,
            "serving": serving,
            "attention": attention,
            "down": down,
            "unknown": unknown,
            "redis_reachable": redis_reachable,
        })
        if redis_error is not UNSET:
            field_dict["redis_error"] = redis_error
        if read_at is not UNSET:
            field_dict["read_at"] = read_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        total = d.pop("total")

        serving = d.pop("serving")

        attention = d.pop("attention")

        down = d.pop("down")

        unknown = d.pop("unknown")

        redis_reachable = d.pop("redis_reachable")

        def _parse_redis_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        redis_error = _parse_redis_error(d.pop("redis_error", UNSET))


        def _parse_read_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                read_at_type_0 = datetime.datetime.fromisoformat(data)



                return read_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        read_at = _parse_read_at(d.pop("read_at", UNSET))


        services_summary = cls(
            total=total,
            serving=serving,
            attention=attention,
            down=down,
            unknown=unknown,
            redis_reachable=redis_reachable,
            redis_error=redis_error,
            read_at=read_at,
        )


        services_summary.additional_properties = d
        return services_summary

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
