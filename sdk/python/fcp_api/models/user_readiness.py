from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.emergency_location_response import EmergencyLocationResponse





T = TypeVar("T", bound="UserReadiness")



@_attrs_define
class UserReadiness:
    """ 
        Attributes:
            user_id (str):
            username (str):
            ready (bool):
            nomadic (bool): Whether every device of this user's moves (a softphone, a mobile or
                a browser), so where it was provisioned may not be where it is.
            issues (list[str]): Why the user is not (fully) ready; empty when ready.
            location (EmergencyLocationResponse | None | Unset):
     """

    user_id: str
    username: str
    ready: bool
    nomadic: bool
    issues: list[str]
    location: EmergencyLocationResponse | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.emergency_location_response import EmergencyLocationResponse # noqa: PLC0415
        user_id = self.user_id

        username = self.username

        ready = self.ready

        nomadic = self.nomadic

        issues = self.issues



        location: dict[str, Any] | None | Unset
        if isinstance(self.location, Unset):
            location = UNSET
        elif isinstance(self.location, EmergencyLocationResponse):
            location = self.location.to_dict()
        else:
            location = self.location


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "user_id": user_id,
            "username": username,
            "ready": ready,
            "nomadic": nomadic,
            "issues": issues,
        })
        if location is not UNSET:
            field_dict["location"] = location

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.emergency_location_response import EmergencyLocationResponse # noqa: PLC0415
        d = dict(src_dict)
        user_id = d.pop("user_id")

        username = d.pop("username")

        ready = d.pop("ready")

        nomadic = d.pop("nomadic")

        issues = cast(list[str], d.pop("issues"))


        def _parse_location(data: object) -> EmergencyLocationResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                location_type_0 = EmergencyLocationResponse.from_dict(data)



                return location_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(EmergencyLocationResponse | None | Unset, data)

        location = _parse_location(d.pop("location", UNSET))


        user_readiness = cls(
            user_id=user_id,
            username=username,
            ready=ready,
            nomadic=nomadic,
            issues=issues,
            location=location,
        )


        user_readiness.additional_properties = d
        return user_readiness

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
