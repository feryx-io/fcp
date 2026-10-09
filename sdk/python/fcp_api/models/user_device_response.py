from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.device_app_response import DeviceAppResponse





T = TypeVar("T", bound="UserDeviceResponse")



@_attrs_define
class UserDeviceResponse:
    """ One of a user's devices.

        Attributes:
            id (str):
            name (str):
            device_type (str):
            registration_state (str):
            model (None | str | Unset):
            sip_username (None | str | Unset):
            last_registration (datetime.datetime | None | Unset):
            user_agent (None | str | Unset):
            push_provider (None | str | Unset): The provider that wakes it when it sleeps (`apns`, `fcm`), while it
                is registered with a push address (`docs/PUSH.md` §2.1).
            app (DeviceAppResponse | None | Unset):
     """

    id: str
    name: str
    device_type: str
    registration_state: str
    model: None | str | Unset = UNSET
    sip_username: None | str | Unset = UNSET
    last_registration: datetime.datetime | None | Unset = UNSET
    user_agent: None | str | Unset = UNSET
    push_provider: None | str | Unset = UNSET
    app: DeviceAppResponse | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.device_app_response import DeviceAppResponse # noqa: PLC0415
        id = self.id

        name = self.name

        device_type = self.device_type

        registration_state = self.registration_state

        model: None | str | Unset
        if isinstance(self.model, Unset):
            model = UNSET
        else:
            model = self.model

        sip_username: None | str | Unset
        if isinstance(self.sip_username, Unset):
            sip_username = UNSET
        else:
            sip_username = self.sip_username

        last_registration: None | str | Unset
        if isinstance(self.last_registration, Unset):
            last_registration = UNSET
        elif isinstance(self.last_registration, datetime.datetime):
            last_registration = self.last_registration.isoformat()
        else:
            last_registration = self.last_registration

        user_agent: None | str | Unset
        if isinstance(self.user_agent, Unset):
            user_agent = UNSET
        else:
            user_agent = self.user_agent

        push_provider: None | str | Unset
        if isinstance(self.push_provider, Unset):
            push_provider = UNSET
        else:
            push_provider = self.push_provider

        app: dict[str, Any] | None | Unset
        if isinstance(self.app, Unset):
            app = UNSET
        elif isinstance(self.app, DeviceAppResponse):
            app = self.app.to_dict()
        else:
            app = self.app


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "device_type": device_type,
            "registration_state": registration_state,
        })
        if model is not UNSET:
            field_dict["model"] = model
        if sip_username is not UNSET:
            field_dict["sip_username"] = sip_username
        if last_registration is not UNSET:
            field_dict["last_registration"] = last_registration
        if user_agent is not UNSET:
            field_dict["user_agent"] = user_agent
        if push_provider is not UNSET:
            field_dict["push_provider"] = push_provider
        if app is not UNSET:
            field_dict["app"] = app

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.device_app_response import DeviceAppResponse # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        device_type = d.pop("device_type")

        registration_state = d.pop("registration_state")

        def _parse_model(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model = _parse_model(d.pop("model", UNSET))


        def _parse_sip_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_username = _parse_sip_username(d.pop("sip_username", UNSET))


        def _parse_last_registration(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_registration_type_0 = datetime.datetime.fromisoformat(data)



                return last_registration_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_registration = _parse_last_registration(d.pop("last_registration", UNSET))


        def _parse_user_agent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        user_agent = _parse_user_agent(d.pop("user_agent", UNSET))


        def _parse_push_provider(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        push_provider = _parse_push_provider(d.pop("push_provider", UNSET))


        def _parse_app(data: object) -> DeviceAppResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                app_type_0 = DeviceAppResponse.from_dict(data)



                return app_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(DeviceAppResponse | None | Unset, data)

        app = _parse_app(d.pop("app", UNSET))


        user_device_response = cls(
            id=id,
            name=name,
            device_type=device_type,
            registration_state=registration_state,
            model=model,
            sip_username=sip_username,
            last_registration=last_registration,
            user_agent=user_agent,
            push_provider=push_provider,
            app=app,
        )


        user_device_response.additional_properties = d
        return user_device_response

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
